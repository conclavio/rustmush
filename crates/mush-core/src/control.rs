//! Rules-neutral exclusive control leases and safe-boundary handoff.
//!
//! A station [`crate::ChannelGrant`] says who is eligible to operate a subject.
//! A [`ControlLeaseState`] says which principal currently has exclusive control
//! of one `(subject, channel)`. Domain/ruleset code chooses controllers,
//! reasons, and safe boundary stamps; this module enforces only bounded shape,
//! checked epochs/revisions, exact typed clocks, and deterministic transitions.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::action::{
    ActionError, ActionErrorCode, AuthorityGuard, AuthorityGuardContext, AuthorityGuardValidator,
    PrincipalId, RequestUid,
};
use crate::action_state::{
    StateFamilyDescriptor, StateFamilyDescriptorError, StateFamilyHandle, StateFamilyId,
    StateKeyCodec, StateValueDecoder, StateValueSerializer, StateValueValidator,
};
use crate::behavior::ChannelId;
use crate::clock::{ClockMismatch, ClockStamp};
use crate::mechanics::{
    CanonicalEncodingError, CanonicalId, OwnerId, canonical_json_bytes_with_limit,
};
use crate::obj::ObjId;

/// Maximum canonical bytes in one control-lease record.
pub const MAX_CONTROL_LEASE_BYTES: usize = 64 * 1024;

/// Exact primary-lease coordinate. There is at most one authoritative record
/// for each `(subject, channel)` pair.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlLeaseKey {
    subject: ObjId,
    channel: ChannelId,
}

impl ControlLeaseKey {
    #[must_use]
    pub const fn new(subject: ObjId, channel: ChannelId) -> Self {
        Self { subject, channel }
    }

    #[must_use]
    pub const fn subject(&self) -> ObjId {
        self.subject
    }

    #[must_use]
    pub const fn channel(&self) -> &ChannelId {
        &self.channel
    }
}

/// Provider-selected request to replace or revoke a primary controller at one
/// exact safe boundary. `next_holder = None` means revoke to vacant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlHandoffDraft {
    pub request: RequestUid,
    pub requested_by: PrincipalId,
    pub next_holder: Option<PrincipalId>,
    pub requested_at: ClockStamp,
    pub boundary: ClockStamp,
    pub reason: CanonicalId,
}

/// Durable pending handoff. The old lease remains active until this record is
/// applied at or after `boundary`; merely requesting a handoff never grants
/// early authority or invalidates already-admitted work.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlHandoff {
    request: RequestUid,
    requested_by: PrincipalId,
    from: Option<PrincipalId>,
    to: Option<PrincipalId>,
    observed_epoch: u64,
    requested_at: ClockStamp,
    boundary: ClockStamp,
    reason: CanonicalId,
}

impl ControlHandoff {
    #[must_use]
    pub const fn request(&self) -> RequestUid {
        self.request
    }

    #[must_use]
    pub const fn requested_by(&self) -> &PrincipalId {
        &self.requested_by
    }

    #[must_use]
    pub const fn from(&self) -> Option<&PrincipalId> {
        self.from.as_ref()
    }

    #[must_use]
    pub const fn to(&self) -> Option<&PrincipalId> {
        self.to.as_ref()
    }

    #[must_use]
    pub const fn observed_epoch(&self) -> u64 {
        self.observed_epoch
    }

    #[must_use]
    pub const fn requested_at(&self) -> &ClockStamp {
        &self.requested_at
    }

    #[must_use]
    pub const fn boundary(&self) -> &ClockStamp {
        &self.boundary
    }

    #[must_use]
    pub const fn reason(&self) -> &CanonicalId {
        &self.reason
    }
}

/// Durable exclusive controller for one exact channel.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlLeaseState {
    schema_version: u16,
    key: ControlLeaseKey,
    holder: Option<PrincipalId>,
    epoch: u64,
    revision: u64,
    changed_at: Option<ClockStamp>,
    last_transition: Option<RequestUid>,
    last_reason: Option<CanonicalId>,
    pending: Option<ControlHandoff>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlLeaseStateWire {
    schema_version: u16,
    key: ControlLeaseKey,
    holder: Option<PrincipalId>,
    epoch: u64,
    revision: u64,
    changed_at: Option<ClockStamp>,
    last_transition: Option<RequestUid>,
    last_reason: Option<CanonicalId>,
    pending: Option<ControlHandoff>,
}

impl<'de> Deserialize<'de> for ControlLeaseState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ControlLeaseStateWire::deserialize(deserializer)?;
        Self::from_parts(wire).map_err(serde::de::Error::custom)
    }
}

impl ControlLeaseState {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn new(key: ControlLeaseKey) -> Result<Self, ControlLeaseError> {
        Self::from_parts(ControlLeaseStateWire {
            schema_version: Self::SCHEMA_VERSION,
            key,
            holder: None,
            epoch: 0,
            revision: 0,
            changed_at: None,
            last_transition: None,
            last_reason: None,
            pending: None,
        })
    }

    fn from_parts(wire: ControlLeaseStateWire) -> Result<Self, ControlLeaseError> {
        let state = Self {
            schema_version: wire.schema_version,
            key: wire.key,
            holder: wire.holder,
            epoch: wire.epoch,
            revision: wire.revision,
            changed_at: wire.changed_at,
            last_transition: wire.last_transition,
            last_reason: wire.last_reason,
            pending: wire.pending,
        };
        state.validate()?;
        Ok(state)
    }

    #[must_use]
    pub const fn key(&self) -> &ControlLeaseKey {
        &self.key
    }

    #[must_use]
    pub const fn holder(&self) -> Option<&PrincipalId> {
        self.holder.as_ref()
    }

    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn changed_at(&self) -> Option<&ClockStamp> {
        self.changed_at.as_ref()
    }

    #[must_use]
    pub const fn last_transition(&self) -> Option<RequestUid> {
        self.last_transition
    }

    #[must_use]
    pub const fn last_reason(&self) -> Option<&CanonicalId> {
        self.last_reason.as_ref()
    }

    #[must_use]
    pub const fn pending(&self) -> Option<&ControlHandoff> {
        self.pending.as_ref()
    }

    /// Request a controller replacement or revocation. The chosen boundary is
    /// policy input, but it must be on the same exact clock generation and may
    /// not precede the observation that requested it.
    pub fn checked_request_handoff(
        &self,
        expected_revision: u64,
        draft: ControlHandoffDraft,
    ) -> Result<Self, ControlLeaseError> {
        self.require_revision(expected_revision)?;
        if let Some(pending) = &self.pending {
            return if pending.request == draft.request {
                Err(ControlLeaseError::DuplicatePending {
                    request: draft.request,
                })
            } else {
                Err(ControlLeaseError::PendingExists {
                    request: pending.request,
                })
            };
        }
        if self.holder == draft.next_holder {
            return Err(ControlLeaseError::NoControlChange);
        }
        require_boundary(&draft.requested_at, &draft.boundary)?;
        let next = Self {
            revision: self.next_revision()?,
            pending: Some(ControlHandoff {
                request: draft.request,
                requested_by: draft.requested_by,
                from: self.holder.clone(),
                to: draft.next_holder,
                observed_epoch: self.epoch,
                requested_at: draft.requested_at,
                boundary: draft.boundary,
                reason: draft.reason,
            }),
            ..self.clone()
        };
        next.checked_post_image()
    }

    /// Cancel one exact still-pending handoff. Authorization is a provider
    /// concern; core only prevents cancelling a different request or revision.
    pub fn checked_cancel_handoff(
        &self,
        expected_revision: u64,
        request: RequestUid,
    ) -> Result<Self, ControlLeaseError> {
        self.require_revision(expected_revision)?;
        let pending = self.pending.as_ref().ok_or(ControlLeaseError::NoPending)?;
        if pending.request != request {
            return Err(ControlLeaseError::WrongPending {
                expected: pending.request,
                actual: request,
            });
        }
        Self {
            revision: self.next_revision()?,
            pending: None,
            ..self.clone()
        }
        .checked_post_image()
    }

    /// Apply one exact handoff at a declared safe boundary. The old holder and
    /// epoch remain authoritative until this transition succeeds; the epoch
    /// increments exactly when the new holder (or vacancy) becomes active.
    pub fn checked_apply_handoff(
        &self,
        expected_revision: u64,
        request: RequestUid,
        now: ClockStamp,
    ) -> Result<Self, ControlLeaseError> {
        self.require_revision(expected_revision)?;
        let pending = self.pending.as_ref().ok_or(ControlLeaseError::NoPending)?;
        if pending.request != request {
            return Err(ControlLeaseError::WrongPending {
                expected: pending.request,
                actual: request,
            });
        }
        if pending.observed_epoch != self.epoch || pending.from != self.holder {
            return Err(ControlLeaseError::PendingStateMismatch);
        }
        match now.checked_cmp(&pending.boundary)? {
            Ordering::Less => return Err(ControlLeaseError::BeforeBoundary),
            Ordering::Equal | Ordering::Greater => {}
        }
        let epoch = self
            .epoch
            .checked_add(1)
            .ok_or(ControlLeaseError::EpochExhausted)?;
        Self {
            holder: pending.to.clone(),
            epoch,
            revision: self.next_revision()?,
            changed_at: Some(now),
            last_transition: Some(pending.request),
            last_reason: Some(pending.reason.clone()),
            pending: None,
            ..self.clone()
        }
        .checked_post_image()
    }

    /// Build the exact private guard carried by work admitted under the active
    /// lease. A vacant channel has no authority to witness.
    #[must_use]
    pub fn authority_guard(&self, provider: OwnerId) -> Option<AuthorityGuard> {
        self.holder
            .as_ref()
            .map(|holder| AuthorityGuard::ControlLease {
                provider,
                subject: self.key.subject,
                channel: self.key.channel.clone(),
                holder: holder.clone(),
                epoch: self.epoch,
            })
    }

    #[must_use]
    pub fn matches_guard(&self, provider: &OwnerId, guard: &AuthorityGuard) -> bool {
        matches!(
            guard,
            AuthorityGuard::ControlLease {
                provider: expected_provider,
                subject,
                channel,
                holder,
                epoch,
            } if expected_provider == provider
                && *subject == self.key.subject
                && channel == &self.key.channel
                && self.holder.as_ref() == Some(holder)
                && *epoch == self.epoch
        )
    }

    pub fn validate(&self) -> Result<(), ControlLeaseError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(ControlLeaseError::UnsupportedSchema {
                actual: self.schema_version,
                supported: Self::SCHEMA_VERSION,
            });
        }
        if self.revision < self.epoch {
            return Err(ControlLeaseError::RevisionBehindEpoch {
                revision: self.revision,
                epoch: self.epoch,
            });
        }
        if self.epoch == 0 {
            if self.holder.is_some()
                || self.changed_at.is_some()
                || self.last_transition.is_some()
                || self.last_reason.is_some()
            {
                return Err(ControlLeaseError::InvalidInitialState);
            }
        } else if self.changed_at.is_none()
            || self.last_transition.is_none()
            || self.last_reason.is_none()
        {
            return Err(ControlLeaseError::MissingTransitionWitness);
        }
        if let Some(pending) = &self.pending {
            if pending.observed_epoch != self.epoch || pending.from != self.holder {
                return Err(ControlLeaseError::PendingStateMismatch);
            }
            if pending.to == self.holder {
                return Err(ControlLeaseError::NoControlChange);
            }
            require_boundary(&pending.requested_at, &pending.boundary)?;
        }
        canonical_json_bytes_with_limit(&json!(self), MAX_CONTROL_LEASE_BYTES)
            .map_err(ControlLeaseError::Encoding)?;
        Ok(())
    }

    fn require_revision(&self, expected: u64) -> Result<(), ControlLeaseError> {
        if self.revision != expected {
            return Err(ControlLeaseError::StaleRevision {
                expected,
                actual: self.revision,
            });
        }
        Ok(())
    }

    fn next_revision(&self) -> Result<u64, ControlLeaseError> {
        self.revision
            .checked_add(1)
            .ok_or(ControlLeaseError::RevisionExhausted)
    }

    fn checked_post_image(self) -> Result<Self, ControlLeaseError> {
        self.validate()?;
        Ok(self)
    }
}

fn require_boundary(
    requested: &ClockStamp,
    boundary: &ClockStamp,
) -> Result<(), ControlLeaseError> {
    if boundary.checked_cmp(requested)? == Ordering::Less {
        return Err(ControlLeaseError::BoundaryBeforeRequest);
    }
    Ok(())
}

/// Invalid durable lease state or checked transition.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ControlLeaseError {
    #[error("unsupported control-lease schema {actual}; supported schema is {supported}")]
    UnsupportedSchema { actual: u16, supported: u16 },
    #[error("control-lease revision {revision} is behind epoch {epoch}")]
    RevisionBehindEpoch { revision: u64, epoch: u64 },
    #[error("epoch zero may not claim an active or historical controller")]
    InvalidInitialState,
    #[error("a nonzero lease epoch is missing its transition witness")]
    MissingTransitionWitness,
    #[error("pending handoff does not match the active holder and epoch")]
    PendingStateMismatch,
    #[error("handoff does not change the active controller")]
    NoControlChange,
    #[error("control-lease revision is stale: expected {expected}, actual {actual}")]
    StaleRevision { expected: u64, actual: u64 },
    #[error("handoff {request} is already pending")]
    DuplicatePending { request: RequestUid },
    #[error("handoff {request} is already pending on this channel")]
    PendingExists { request: RequestUid },
    #[error("there is no pending handoff")]
    NoPending,
    #[error("wrong pending handoff: expected {expected}, got {actual}")]
    WrongPending {
        expected: RequestUid,
        actual: RequestUid,
    },
    #[error("handoff boundary precedes its request")]
    BoundaryBeforeRequest,
    #[error("handoff has not reached its safe boundary")]
    BeforeBoundary,
    #[error("control-lease epoch is exhausted")]
    EpochExhausted,
    #[error("control-lease revision is exhausted")]
    RevisionExhausted,
    #[error(transparent)]
    Clock(#[from] ClockMismatch),
    #[error("control lease cannot be canonically encoded: {0}")]
    Encoding(CanonicalEncodingError),
}

/// Shared codec for a module-owned authoritative lease family.
#[derive(Clone, Copy, Debug, Default)]
pub struct ControlLeaseStateCodec;

impl StateKeyCodec<ControlLeaseKey> for ControlLeaseStateCodec {
    fn encode_key(&self, key: &ControlLeaseKey) -> Result<String, String> {
        Ok(format!("{}:{}", key.subject.0, key.channel.as_str()))
    }
}

impl StateValueDecoder<ControlLeaseState> for ControlLeaseStateCodec {
    fn decode_value(&self, value: &serde_json::Value) -> Result<ControlLeaseState, String> {
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())
    }
}

impl StateValueValidator<ControlLeaseState> for ControlLeaseStateCodec {
    fn validate_value(&self, value: &ControlLeaseState) -> Result<(), String> {
        value.validate().map_err(|error| error.to_string())
    }
}

impl StateValueSerializer<ControlLeaseState> for ControlLeaseStateCodec {
    fn serialize_value(&self, value: &ControlLeaseState) -> Result<serde_json::Value, String> {
        serde_json::to_value(value).map_err(|error| error.to_string())
    }
}

/// Build the reusable typed-family descriptor for module-owned control leases.
pub fn control_lease_state_family_descriptor(
    family: StateFamilyId,
) -> Result<StateFamilyDescriptor, StateFamilyDescriptorError> {
    StateFamilyDescriptor::new(
        family,
        CanonicalId::new("control_lease_key.v1").expect("static canonical id"),
        CanonicalId::new("control_lease_state.v1").expect("static canonical id"),
        ControlLeaseState::SCHEMA_VERSION,
        CanonicalId::new("control_lease_decode.v1").expect("static canonical id"),
        CanonicalId::new("control_lease_validate.v1").expect("static canonical id"),
        CanonicalId::new("control_lease_serialize.v1").expect("static canonical id"),
    )?
    .with_limits(256, MAX_CONTROL_LEASE_BYTES)
}

/// Generic typed-family validator for [`AuthorityGuard::ControlLease`]. A
/// module registers this under `ControlLease`; its composition-branded handle
/// makes another owner/family incapable of satisfying the guard.
#[derive(Clone)]
pub struct ControlLeaseGuardValidator {
    leases: StateFamilyHandle<ControlLeaseKey, ControlLeaseState>,
}

impl fmt::Debug for ControlLeaseGuardValidator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlLeaseGuardValidator")
            .field("owner", self.leases.owner())
            .field("family", self.leases.descriptor().id())
            .finish()
    }
}

impl ControlLeaseGuardValidator {
    #[must_use]
    pub const fn new(leases: StateFamilyHandle<ControlLeaseKey, ControlLeaseState>) -> Self {
        Self { leases }
    }
}

impl AuthorityGuardValidator for ControlLeaseGuardValidator {
    fn is_current(
        &self,
        context: &AuthorityGuardContext<'_>,
        guard: &AuthorityGuard,
    ) -> Result<bool, ActionError> {
        let AuthorityGuard::ControlLease {
            provider,
            subject,
            channel,
            ..
        } = guard
        else {
            return Ok(false);
        };
        if provider != self.leases.owner() {
            return Ok(false);
        }
        let key = ControlLeaseKey::new(*subject, channel.clone());
        let current = context.action_state(&self.leases, &key).map_err(|_| {
            ActionError::new(
                ActionErrorCode::Integrity,
                false,
                "control-lease authority state is unavailable",
            )
            .expect("static control-lease read error is bounded")
        })?;
        Ok(current.is_some_and(|lease| lease.matches_guard(provider, guard)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use crate::action_state::{ActionStateStore, StateFamilyRegistryBuilder};
    use crate::clock::ClockRef;
    use crate::mechanics::MechanicalOwnerReadiness;
    use crate::world::World;

    fn actor(id: u32) -> PrincipalId {
        PrincipalId::Object { id: ObjId(id) }
    }

    fn request(byte: u8) -> RequestUid {
        RequestUid::from_bytes([byte; 16])
    }

    fn tick(at: i64, generation: u64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::DomainTick {
                owner: OwnerId::new("sim").unwrap(),
                id: CanonicalId::new("battlefield").unwrap(),
                generation,
            },
            at,
        }
    }

    fn draft(
        id: u8,
        from_time: i64,
        boundary: i64,
        next_holder: Option<PrincipalId>,
    ) -> ControlHandoffDraft {
        ControlHandoffDraft {
            request: request(id),
            requested_by: actor(1),
            next_holder,
            requested_at: tick(from_time, 3),
            boundary: tick(boundary, 3),
            reason: CanonicalId::new("operator_change").unwrap(),
        }
    }

    fn vacant() -> ControlLeaseState {
        ControlLeaseState::new(ControlLeaseKey::new(
            ObjId(20),
            ChannelId::new("movement").unwrap(),
        ))
        .unwrap()
    }

    #[test]
    fn handoff_activates_only_at_the_exact_safe_boundary() {
        let vacant = vacant();
        let pending = vacant
            .checked_request_handoff(0, draft(1, 10, 12, Some(actor(7))))
            .unwrap();
        assert_eq!(pending.epoch(), 0);
        assert!(pending.holder().is_none());
        assert_eq!(pending.revision(), 1);
        assert_eq!(
            pending.checked_apply_handoff(1, request(1), tick(11, 3)),
            Err(ControlLeaseError::BeforeBoundary)
        );
        assert!(matches!(
            pending.checked_apply_handoff(1, request(1), tick(12, 4)),
            Err(ControlLeaseError::Clock(_))
        ));

        let active = pending
            .checked_apply_handoff(1, request(1), tick(12, 3))
            .unwrap();
        assert_eq!(active.holder(), Some(&actor(7)));
        assert_eq!(active.epoch(), 1);
        assert_eq!(active.revision(), 2);
        assert_eq!(active.last_transition(), Some(request(1)));
        assert!(active.pending().is_none());
    }

    #[test]
    fn request_cancel_and_stale_checks_are_non_mutating() {
        let vacant = vacant();
        assert_eq!(
            vacant.checked_request_handoff(0, draft(1, 12, 11, Some(actor(7)))),
            Err(ControlLeaseError::BoundaryBeforeRequest)
        );
        assert_eq!(
            vacant.checked_request_handoff(9, draft(1, 10, 11, Some(actor(7)))),
            Err(ControlLeaseError::StaleRevision {
                expected: 9,
                actual: 0
            })
        );
        let pending = vacant
            .checked_request_handoff(0, draft(1, 10, 11, Some(actor(7))))
            .unwrap();
        assert!(matches!(
            pending.checked_request_handoff(1, draft(2, 10, 11, Some(actor(8)))),
            Err(ControlLeaseError::PendingExists { .. })
        ));
        assert!(matches!(
            pending.checked_cancel_handoff(1, request(2)),
            Err(ControlLeaseError::WrongPending { .. })
        ));
        let cancelled = pending.checked_cancel_handoff(1, request(1)).unwrap();
        assert_eq!(cancelled.revision(), 2);
        assert_eq!(cancelled.epoch(), 0);
        assert!(cancelled.pending().is_none());
    }

    #[test]
    fn revocation_and_reacquisition_advance_epoch_and_invalidate_guard() {
        let pending = vacant()
            .checked_request_handoff(0, draft(1, 1, 2, Some(actor(7))))
            .unwrap();
        let active = pending
            .checked_apply_handoff(1, request(1), tick(2, 3))
            .unwrap();
        let provider = OwnerId::new("sim").unwrap();
        let old_guard = active.authority_guard(provider.clone()).unwrap();
        assert!(active.matches_guard(&provider, &old_guard));

        let pending_revoke = active
            .checked_request_handoff(2, draft(2, 3, 4, None))
            .unwrap();
        assert!(pending_revoke.matches_guard(&provider, &old_guard));
        let revoked = pending_revoke
            .checked_apply_handoff(3, request(2), tick(4, 3))
            .unwrap();
        assert_eq!(revoked.epoch(), 2);
        assert!(revoked.holder().is_none());
        assert!(revoked.authority_guard(provider.clone()).is_none());
        assert!(!revoked.matches_guard(&provider, &old_guard));

        let reacquire_pending = revoked
            .checked_request_handoff(4, draft(3, 5, 5, Some(actor(8))))
            .unwrap();
        let reacquired = reacquire_pending
            .checked_apply_handoff(5, request(3), tick(5, 3))
            .unwrap();
        assert_eq!(reacquired.epoch(), 3);
        assert_eq!(reacquired.holder(), Some(&actor(8)));
    }

    #[test]
    fn strict_decode_rejects_tampered_epoch_pending_and_unknown_fields() {
        let active = vacant()
            .checked_request_handoff(0, draft(1, 1, 2, Some(actor(7))))
            .unwrap()
            .checked_apply_handoff(1, request(1), tick(2, 3))
            .unwrap();
        let value = serde_json::to_value(&active).unwrap();
        assert_eq!(
            serde_json::from_value::<ControlLeaseState>(value.clone()).unwrap(),
            active
        );

        let mut revision = value.clone();
        revision["revision"] = json!(0);
        assert!(serde_json::from_value::<ControlLeaseState>(revision).is_err());

        let mut initial = value.clone();
        initial["epoch"] = json!(0);
        assert!(serde_json::from_value::<ControlLeaseState>(initial).is_err());

        let mut unknown = value;
        unknown["extra"] = json!(true);
        assert!(serde_json::from_value::<ControlLeaseState>(unknown).is_err());
    }

    #[test]
    fn codec_and_family_identity_are_stable_and_bounded() {
        let state = vacant();
        let codec = ControlLeaseStateCodec;
        assert_eq!(
            codec.encode_key(state.key()).unwrap(),
            "20:movement".to_owned()
        );
        let encoded = codec.serialize_value(&state).unwrap();
        assert_eq!(codec.decode_value(&encoded).unwrap(), state);
        let descriptor =
            control_lease_state_family_descriptor(StateFamilyId::new("control_leases").unwrap())
                .unwrap();
        assert_eq!(descriptor.schema_version(), 1);
        assert_eq!(descriptor.max_record_bytes(), MAX_CONTROL_LEASE_BYTES);
    }

    #[test]
    fn registered_validator_reads_the_exact_installed_typed_family() {
        let provider = OwnerId::new("sim").unwrap();
        let owners = BTreeSet::from([provider.clone()]);
        let readiness = MechanicalOwnerReadiness::for_owners(&owners);
        let mut families = StateFamilyRegistryBuilder::new();
        let codec = ControlLeaseStateCodec;
        let handle = families
            .register(
                provider.clone(),
                control_lease_state_family_descriptor(
                    StateFamilyId::new("control_leases").unwrap(),
                )
                .unwrap(),
                codec,
                codec,
                codec,
                codec,
            )
            .unwrap();
        let registry = families
            .freeze_for_owners_with_readiness(&owners, readiness)
            .unwrap();
        let mut store = ActionStateStore::new(registry);
        let active = vacant()
            .checked_request_handoff(0, draft(1, 1, 2, Some(actor(7))))
            .unwrap()
            .checked_apply_handoff(1, request(1), tick(2, 3))
            .unwrap();
        let guard = active.authority_guard(provider).unwrap();

        let mut transaction = store.transaction(&handle).unwrap();
        transaction
            .insert(&handle, active.key(), active.clone())
            .unwrap();
        let (_, installation) = transaction.prepare().unwrap().into_parts();
        store.install(installation);

        let world = World::new_minimal();
        let context = AuthorityGuardContext::new(&world, &store);
        let validator = ControlLeaseGuardValidator::new(handle);
        assert!(validator.is_current(&context, &guard).unwrap());

        let AuthorityGuard::ControlLease {
            provider,
            subject,
            channel,
            holder,
            epoch,
        } = guard
        else {
            unreachable!("active lease always produces a control-lease guard")
        };
        let stale = AuthorityGuard::ControlLease {
            provider,
            subject,
            channel,
            holder,
            epoch: epoch + 1,
        };
        assert!(!validator.is_current(&context, &stale).unwrap());
    }
}
