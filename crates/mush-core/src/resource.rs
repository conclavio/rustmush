//! Rules-neutral checked resources, reservations, and effect instances.
//!
//! These values standardize arithmetic, identity, provenance, clocks, and
//! lifecycle validation. They deliberately do not choose game meaning or a
//! universal storage table: a rules module still owns its typed state family
//! and decides which operations and stacking policies are legal.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::action::{
    CanonicalHash, CanonicalHashError, CommitToken, PreparedCausalRef, RequestUid,
    ScopedIdempotencyKey, SubjectKey,
};
use crate::clock::{ClockMismatch, ClockStamp};
use crate::mechanics::{CanonicalId, DefinitionRef, OwnerId, RulesValue};

/// Maximum live reservations considered for one checked availability result.
pub const MAX_RESOURCE_RESERVATIONS: usize = 1_024;
/// Structural ceiling for one effect's stack count. Rulesets may impose a
/// lower limit or refuse stacking entirely.
pub const MAX_EFFECT_STACKS: u16 = 1_024;

/// Fully qualified identity of one ruleset-owned resource or track.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceRef {
    pub owner: OwnerId,
    pub subject: SubjectKey,
    pub id: CanonicalId,
}

impl ResourceRef {
    #[must_use]
    pub const fn new(owner: OwnerId, subject: SubjectKey, id: CanonicalId) -> Self {
        Self { owner, subject, id }
    }
}

/// A bounded integer with an exact monotonic revision.
///
/// Deserialization rechecks all invariants so malformed persisted state can
/// never become a valid scalar merely by bypassing [`Self::new`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoundedScalar {
    minimum: i64,
    maximum: i64,
    current: i64,
    revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundedScalarWire {
    minimum: i64,
    maximum: i64,
    current: i64,
    revision: u64,
}

impl<'de> Deserialize<'de> for BoundedScalar {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = BoundedScalarWire::deserialize(deserializer)?;
        Self::new(wire.minimum, wire.maximum, wire.current, wire.revision)
            .map_err(serde::de::Error::custom)
    }
}

impl BoundedScalar {
    pub fn new(
        minimum: i64,
        maximum: i64,
        current: i64,
        revision: u64,
    ) -> Result<Self, ResourceValueError> {
        if minimum > maximum {
            return Err(ResourceValueError::InvalidBounds { minimum, maximum });
        }
        if !(minimum..=maximum).contains(&current) {
            return Err(ResourceValueError::OutOfBounds {
                minimum,
                maximum,
                attempted: current,
            });
        }
        Ok(Self {
            minimum,
            maximum,
            current,
            revision,
        })
    }

    #[must_use]
    pub const fn minimum(self) -> i64 {
        self.minimum
    }

    #[must_use]
    pub const fn maximum(self) -> i64 {
        self.maximum
    }

    #[must_use]
    pub const fn current(self) -> i64 {
        self.current
    }

    #[must_use]
    pub const fn revision(self) -> u64 {
        self.revision
    }

    /// Apply one operation with checked arithmetic, bounds, and revision.
    pub fn checked_apply(self, operation: ResourceOperation) -> Result<Self, ResourceValueError> {
        let attempted = match operation {
            ResourceOperation::Increase { amount } => {
                let amount = checked_amount(amount)?;
                self.current
                    .checked_add(amount)
                    .ok_or(ResourceValueError::ArithmeticOverflow)?
            }
            ResourceOperation::Decrease { amount } => {
                let amount = checked_amount(amount)?;
                self.current
                    .checked_sub(amount)
                    .ok_or(ResourceValueError::ArithmeticOverflow)?
            }
            ResourceOperation::Set { value } => {
                if value == self.current {
                    return Err(ResourceValueError::NoChange);
                }
                value
            }
        };
        if !(self.minimum..=self.maximum).contains(&attempted) {
            return Err(ResourceValueError::OutOfBounds {
                minimum: self.minimum,
                maximum: self.maximum,
                attempted,
            });
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(ResourceValueError::RevisionExhausted)?;
        Ok(Self {
            current: attempted,
            revision,
            ..self
        })
    }

    /// Amount available above the scalar's minimum after exact live holds.
    pub fn checked_available(
        self,
        held: impl IntoIterator<Item = u64>,
    ) -> Result<u64, ResourceValueError> {
        let mut total = 0_u64;
        for (count, amount) in held.into_iter().enumerate() {
            if count >= MAX_RESOURCE_RESERVATIONS {
                return Err(ResourceValueError::ReservationLimit {
                    max: MAX_RESOURCE_RESERVATIONS,
                });
            }
            if amount == 0 {
                return Err(ResourceValueError::ZeroAmount);
            }
            total = total
                .checked_add(amount)
                .ok_or(ResourceValueError::ArithmeticOverflow)?;
        }
        let capacity = self
            .current
            .checked_sub(self.minimum)
            .ok_or(ResourceValueError::ArithmeticOverflow)?;
        let capacity =
            u64::try_from(capacity).map_err(|_| ResourceValueError::ArithmeticOverflow)?;
        capacity
            .checked_sub(total)
            .ok_or(ResourceValueError::InsufficientAvailable {
                available: capacity,
                requested: total,
            })
    }
}

/// The kernel's complete arithmetic vocabulary. Game-specific labels and
/// permission to use an operation remain provider-owned.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceOperation {
    Increase { amount: u64 },
    Decrease { amount: u64 },
    Set { value: i64 },
}

fn checked_amount(amount: u64) -> Result<i64, ResourceValueError> {
    if amount == 0 {
        return Err(ResourceValueError::ZeroAmount);
    }
    i64::try_from(amount).map_err(|_| ResourceValueError::ArithmeticOverflow)
}

/// Checked resource arithmetic or availability failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ResourceValueError {
    #[error("resource minimum {minimum} exceeds maximum {maximum}")]
    InvalidBounds { minimum: i64, maximum: i64 },
    #[error("resource value {attempted} is outside {minimum}..={maximum}")]
    OutOfBounds {
        minimum: i64,
        maximum: i64,
        attempted: i64,
    },
    #[error("resource operations require a non-zero amount")]
    ZeroAmount,
    #[error("resource operation would not change the value")]
    NoChange,
    #[error("resource arithmetic overflow")]
    ArithmeticOverflow,
    #[error("resource revision is exhausted")]
    RevisionExhausted,
    #[error("resource availability considers at most {max} reservations")]
    ReservationLimit { max: usize },
    #[error("resource has {available} available but reservations require {requested}")]
    InsufficientAvailable { available: u64, requested: u64 },
}

/// Stable reservation identity derived by the owning provider.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ReservationId(pub CanonicalHash);

impl ReservationId {
    /// Derive a stable identity from the holder and resource rather than an
    /// allocator whose retry behavior could diverge after restart.
    pub fn derive(
        holder: &ScopedIdempotencyKey,
        resource: &ResourceRef,
    ) -> Result<Self, CanonicalHashError> {
        CanonicalHash::for_canonical_value(
            "resource_reservation.v1",
            1,
            &json!({ "holder": holder, "resource": resource }),
            16 * 1024,
        )
        .map(Self)
    }
}

/// Durable reservation lifecycle. Terminal records remain auditable; their
/// amounts no longer reduce resource availability.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReservationState {
    Held,
    Consumed { commit: CommitToken },
    Released,
    Expired,
}

/// One checked claim on ruleset-owned consumable value or capacity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReservationRecord {
    id: ReservationId,
    owner: OwnerId,
    resource: ResourceRef,
    holder: ScopedIdempotencyKey,
    amount: u64,
    revision: u64,
    expires: ClockStamp,
    state: ReservationState,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReservationRecordWire {
    id: ReservationId,
    owner: OwnerId,
    resource: ResourceRef,
    holder: ScopedIdempotencyKey,
    amount: u64,
    revision: u64,
    expires: ClockStamp,
    state: ReservationState,
}

impl<'de> Deserialize<'de> for ReservationRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ReservationRecordWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.id,
            wire.owner,
            wire.resource,
            wire.holder,
            wire.amount,
            wire.revision,
            wire.expires,
            wire.state,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ReservationRecord {
    pub fn held(
        id: ReservationId,
        resource: ResourceRef,
        holder: ScopedIdempotencyKey,
        amount: u64,
        expires: ClockStamp,
    ) -> Result<Self, ReservationError> {
        Self::from_parts(
            id,
            resource.owner.clone(),
            resource,
            holder,
            amount,
            1,
            expires,
            ReservationState::Held,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        id: ReservationId,
        owner: OwnerId,
        resource: ResourceRef,
        holder: ScopedIdempotencyKey,
        amount: u64,
        revision: u64,
        expires: ClockStamp,
        state: ReservationState,
    ) -> Result<Self, ReservationError> {
        if owner != resource.owner {
            return Err(ReservationError::OwnerMismatch);
        }
        if amount == 0 {
            return Err(ReservationError::ZeroAmount);
        }
        if revision == 0 {
            return Err(ReservationError::ZeroRevision);
        }
        if ReservationId::derive(&holder, &resource).map_err(ReservationError::Identity)? != id {
            return Err(ReservationError::IdentityMismatch);
        }
        Ok(Self {
            id,
            owner,
            resource,
            holder,
            amount,
            revision,
            expires,
            state,
        })
    }

    #[must_use]
    pub const fn id(&self) -> ReservationId {
        self.id
    }

    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn resource(&self) -> &ResourceRef {
        &self.resource
    }

    #[must_use]
    pub const fn holder(&self) -> &ScopedIdempotencyKey {
        &self.holder
    }

    #[must_use]
    pub const fn amount(&self) -> u64 {
        self.amount
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn expires(&self) -> &ClockStamp {
        &self.expires
    }

    #[must_use]
    pub const fn state(&self) -> ReservationState {
        self.state
    }

    #[must_use]
    pub fn held_amount(&self) -> Option<u64> {
        matches!(self.state, ReservationState::Held).then_some(self.amount)
    }

    pub fn checked_replace(
        &self,
        expected_revision: u64,
        amount: u64,
        expires: ClockStamp,
    ) -> Result<Self, ReservationError> {
        self.require_held(expected_revision)?;
        if amount == 0 {
            return Err(ReservationError::ZeroAmount);
        }
        Ok(Self {
            amount,
            expires,
            revision: self.next_revision()?,
            ..self.clone()
        })
    }

    pub fn checked_consume(
        &self,
        expected_revision: u64,
        commit: CommitToken,
    ) -> Result<Self, ReservationError> {
        self.checked_terminal(expected_revision, ReservationState::Consumed { commit })
    }

    pub fn checked_release(&self, expected_revision: u64) -> Result<Self, ReservationError> {
        self.checked_terminal(expected_revision, ReservationState::Released)
    }

    pub fn checked_expire(
        &self,
        expected_revision: u64,
        now: &ClockStamp,
    ) -> Result<Self, ReservationError> {
        self.require_held(expected_revision)?;
        if self.expires.checked_cmp(now)? == std::cmp::Ordering::Greater {
            return Err(ReservationError::NotExpired);
        }
        self.checked_terminal(expected_revision, ReservationState::Expired)
    }

    fn checked_terminal(
        &self,
        expected_revision: u64,
        state: ReservationState,
    ) -> Result<Self, ReservationError> {
        self.require_held(expected_revision)?;
        Ok(Self {
            revision: self.next_revision()?,
            state,
            ..self.clone()
        })
    }

    fn require_held(&self, expected_revision: u64) -> Result<(), ReservationError> {
        if self.revision != expected_revision {
            return Err(ReservationError::StaleRevision {
                expected: expected_revision,
                actual: self.revision,
            });
        }
        if self.state != ReservationState::Held {
            return Err(ReservationError::NotHeld { state: self.state });
        }
        Ok(())
    }

    fn next_revision(&self) -> Result<u64, ReservationError> {
        self.revision
            .checked_add(1)
            .ok_or(ReservationError::RevisionExhausted)
    }
}

/// Invalid reservation record or transition.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ReservationError {
    #[error("reservation owner does not own its resource")]
    OwnerMismatch,
    #[error("reservation amount must be non-zero")]
    ZeroAmount,
    #[error("reservation revision zero is reserved")]
    ZeroRevision,
    #[error("reservation identity is not derived from its holder and resource")]
    IdentityMismatch,
    #[error("reservation identity cannot be derived: {0}")]
    Identity(#[from] CanonicalHashError),
    #[error("reservation revision is stale: expected {expected}, actual {actual}")]
    StaleRevision { expected: u64, actual: u64 },
    #[error("reservation is not held: {state:?}")]
    NotHeld { state: ReservationState },
    #[error("reservation deadline has not elapsed")]
    NotExpired,
    #[error("reservation clock mismatch: {0}")]
    Clock(#[from] ClockMismatch),
    #[error("reservation revision is exhausted")]
    RevisionExhausted,
}

/// Calculate availability from a bounded, duplicate-free set of exact
/// reservation records. Passing a record for another resource is an error,
/// not a silently ignored caller bug.
pub fn checked_resource_availability<'a>(
    scalar: BoundedScalar,
    resource: &ResourceRef,
    reservations: impl IntoIterator<Item = &'a ReservationRecord>,
) -> Result<u64, ReservationAvailabilityError> {
    let mut ids = BTreeSet::new();
    let mut held = Vec::new();
    for (count, reservation) in reservations.into_iter().enumerate() {
        if count >= MAX_RESOURCE_RESERVATIONS {
            return Err(ReservationAvailabilityError::Limit {
                max: MAX_RESOURCE_RESERVATIONS,
            });
        }
        if reservation.resource() != resource {
            return Err(ReservationAvailabilityError::ResourceMismatch {
                reservation: reservation.id(),
            });
        }
        if !ids.insert(reservation.id()) {
            return Err(ReservationAvailabilityError::Duplicate {
                reservation: reservation.id(),
            });
        }
        if let Some(amount) = reservation.held_amount() {
            held.push(amount);
        }
    }
    scalar
        .checked_available(held)
        .map_err(ReservationAvailabilityError::Resource)
}

/// Invalid reservation set supplied to an availability check.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ReservationAvailabilityError {
    #[error("availability considers at most {max} reservation records")]
    Limit { max: usize },
    #[error("reservation {reservation:?} belongs to another resource")]
    ResourceMismatch { reservation: ReservationId },
    #[error("reservation {reservation:?} was supplied more than once")]
    Duplicate { reservation: ReservationId },
    #[error(transparent)]
    Resource(#[from] ResourceValueError),
}

/// Stable effect identity. Providers should derive it from durable causal
/// input, never from process-local allocation order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct EffectInstanceId(pub CanonicalHash);

impl EffectInstanceId {
    pub fn derive(
        root: RequestUid,
        definition: &DefinitionRef,
        target: &EffectTarget,
        ordinal: u32,
    ) -> Result<Self, CanonicalHashError> {
        CanonicalHash::for_canonical_value(
            "effect_instance.v1",
            1,
            &json!({
                "root": root,
                "definition": definition,
                "target": target,
                "ordinal": ordinal,
            }),
            32 * 1024,
        )
        .map(Self)
    }
}

/// Rules-neutral address at which an effect is attached.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectTarget {
    Subject { subject: SubjectKey },
    Resource { resource: ResourceRef },
}

/// Kernel-recognized effect lifetime. The provider owns all other duration
/// and stacking policy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectDuration {
    Permanent,
    Until { expires: ClockStamp },
}

/// Persisted inert effect data with exact rules identity and provenance.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectInstance {
    id: EffectInstanceId,
    definition: DefinitionRef,
    owner: OwnerId,
    source: PreparedCausalRef,
    target: EffectTarget,
    applied_at: ClockStamp,
    duration: EffectDuration,
    stacks: u16,
    revision: u64,
    state: RulesValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectInstanceWire {
    id: EffectInstanceId,
    definition: DefinitionRef,
    owner: OwnerId,
    source: PreparedCausalRef,
    target: EffectTarget,
    applied_at: ClockStamp,
    duration: EffectDuration,
    stacks: u16,
    revision: u64,
    state: RulesValue,
}

impl<'de> Deserialize<'de> for EffectInstance {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = EffectInstanceWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.id,
            wire.definition,
            wire.owner,
            wire.source,
            wire.target,
            wire.applied_at,
            wire.duration,
            wire.stacks,
            wire.revision,
            wire.state,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl EffectInstance {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: EffectInstanceId,
        definition: DefinitionRef,
        source: PreparedCausalRef,
        target: EffectTarget,
        applied_at: ClockStamp,
        duration: EffectDuration,
        stacks: u16,
        state: RulesValue,
    ) -> Result<Self, EffectInstanceError> {
        let owner = definition.key().owner.clone();
        Self::from_parts(
            id, definition, owner, source, target, applied_at, duration, stacks, 1, state,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        id: EffectInstanceId,
        definition: DefinitionRef,
        owner: OwnerId,
        source: PreparedCausalRef,
        target: EffectTarget,
        applied_at: ClockStamp,
        duration: EffectDuration,
        stacks: u16,
        revision: u64,
        state: RulesValue,
    ) -> Result<Self, EffectInstanceError> {
        if definition.key().kind.as_str() != "effect" {
            return Err(EffectInstanceError::DefinitionKind);
        }
        if owner != definition.key().owner || owner != state.schema().key().owner {
            return Err(EffectInstanceError::OwnerMismatch);
        }
        if stacks == 0 || stacks > MAX_EFFECT_STACKS {
            return Err(EffectInstanceError::InvalidStacks {
                actual: stacks,
                max: MAX_EFFECT_STACKS,
            });
        }
        if revision == 0 {
            return Err(EffectInstanceError::ZeroRevision);
        }
        if let EffectDuration::Until { expires } = &duration {
            match applied_at.checked_cmp(expires)? {
                std::cmp::Ordering::Less => {}
                std::cmp::Ordering::Equal | std::cmp::Ordering::Greater => {
                    return Err(EffectInstanceError::InvalidDuration);
                }
            }
        }
        Ok(Self {
            id,
            definition,
            owner,
            source,
            target,
            applied_at,
            duration,
            stacks,
            revision,
            state,
        })
    }

    #[must_use]
    pub const fn id(&self) -> EffectInstanceId {
        self.id
    }

    #[must_use]
    pub const fn definition(&self) -> &DefinitionRef {
        &self.definition
    }

    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn source(&self) -> PreparedCausalRef {
        self.source
    }

    #[must_use]
    pub const fn target(&self) -> &EffectTarget {
        &self.target
    }

    #[must_use]
    pub const fn applied_at(&self) -> &ClockStamp {
        &self.applied_at
    }

    #[must_use]
    pub const fn duration(&self) -> &EffectDuration {
        &self.duration
    }

    #[must_use]
    pub const fn stacks(&self) -> u16 {
        self.stacks
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn state(&self) -> &RulesValue {
        &self.state
    }

    /// Determine expiry only on the exact clock generation named by the
    /// effect. Permanent effects never inspect the supplied clock.
    pub fn is_expired_at(&self, now: &ClockStamp) -> Result<bool, ClockMismatch> {
        match &self.duration {
            EffectDuration::Permanent => Ok(false),
            EffectDuration::Until { expires } => Ok(matches!(
                expires.checked_cmp(now)?,
                std::cmp::Ordering::Less | std::cmp::Ordering::Equal
            )),
        }
    }
}

/// Invalid inert effect record.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum EffectInstanceError {
    #[error("effect instance definition is not an effect definition")]
    DefinitionKind,
    #[error("effect definition, owner, and state schema owners do not agree")]
    OwnerMismatch,
    #[error("effect stack count {actual} is outside 1..={max}")]
    InvalidStacks { actual: u16, max: u16 },
    #[error("effect revision zero is reserved")]
    ZeroRevision,
    #[error("effect expiry must be later than its application stamp")]
    InvalidDuration,
    #[error("effect clock mismatch: {0}")]
    Clock(#[from] ClockMismatch),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{IdempotencyKey, IdempotencyScope, PrincipalId};
    use crate::clock::ClockRef;
    use crate::mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
    };
    use serde_json::Value;

    fn owner() -> OwnerId {
        OwnerId::new("toy").unwrap()
    }

    fn subject() -> SubjectKey {
        SubjectKey::Object {
            id: crate::obj::ObjId(1),
        }
    }

    fn resource() -> ResourceRef {
        ResourceRef::new(owner(), subject(), CanonicalId::new("focus").unwrap())
    }

    fn holder() -> ScopedIdempotencyKey {
        let request = RequestUid::from_bytes([7; 16]);
        ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: PrincipalId::Object {
                    id: crate::obj::ObjId(1),
                },
                owner: owner(),
                namespace: CanonicalId::new("brace").unwrap(),
            },
            0,
            IdempotencyKey::Request(request),
        )
    }

    fn turn(at: i64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::SceneTurn {
                scene: 9,
                generation: 2,
            },
            at,
        }
    }

    fn effect_definition() -> DefinitionRef {
        let mut builder = DefinitionRegistryBuilder::new();
        builder
            .register(
                owner(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("effect").unwrap(),
                    DefinitionId::new("guarded").unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap()
    }

    #[test]
    fn scalar_operations_check_bounds_overflow_and_revision() {
        let scalar = BoundedScalar::new(0, 3, 1, 8).unwrap();
        let spent = scalar
            .checked_apply(ResourceOperation::Decrease { amount: 1 })
            .unwrap();
        assert_eq!(spent.current(), 0);
        assert_eq!(spent.revision(), 9);
        assert!(matches!(
            spent.checked_apply(ResourceOperation::Decrease { amount: 1 }),
            Err(ResourceValueError::OutOfBounds { .. })
        ));
        assert!(
            serde_json::from_value::<BoundedScalar>(json!({
                "minimum": 0,
                "maximum": 3,
                "current": 4,
                "revision": 1
            }))
            .is_err()
        );
    }

    #[test]
    fn availability_subtracts_only_explicit_checked_holds() {
        let scalar = BoundedScalar::new(0, 8, 5, 1).unwrap();
        assert_eq!(scalar.checked_available([2, 1]).unwrap(), 2);
        assert!(matches!(
            scalar.checked_available([4, 2]),
            Err(ResourceValueError::InsufficientAvailable { .. })
        ));
    }

    #[test]
    fn reservation_availability_checks_identity_and_ignores_terminal_holds() {
        let scalar = BoundedScalar::new(0, 8, 5, 1).unwrap();
        let id = ReservationId::derive(&holder(), &resource()).unwrap();
        let held = ReservationRecord::held(id, resource(), holder(), 2, turn(12)).unwrap();
        assert_eq!(
            checked_resource_availability(scalar, &resource(), [&held]).unwrap(),
            3
        );
        let released = held.checked_release(1).unwrap();
        assert_eq!(
            checked_resource_availability(scalar, &resource(), [&released]).unwrap(),
            5
        );
        assert!(matches!(
            checked_resource_availability(scalar, &resource(), [&held, &held]),
            Err(ReservationAvailabilityError::Duplicate { .. })
        ));
    }

    #[test]
    fn reservation_transitions_are_revision_and_clock_checked() {
        let id = ReservationId::derive(&holder(), &resource()).unwrap();
        let held = ReservationRecord::held(id, resource(), holder(), 1, turn(12)).unwrap();
        assert_eq!(held.held_amount(), Some(1));
        assert!(matches!(
            held.checked_expire(1, &turn(11)),
            Err(ReservationError::NotExpired)
        ));
        let expired = held.checked_expire(1, &turn(12)).unwrap();
        assert_eq!(expired.state(), ReservationState::Expired);
        assert_eq!(expired.revision(), 2);
        assert!(matches!(
            expired.checked_release(1),
            Err(ReservationError::StaleRevision { .. })
        ));
    }

    #[test]
    fn effect_duration_is_typed_and_strictly_future() {
        let definition = effect_definition();
        let target = EffectTarget::Subject { subject: subject() };
        let id = EffectInstanceId::derive(RequestUid::from_bytes([4; 16]), &definition, &target, 1)
            .unwrap();
        let state = RulesValue::new(definition.clone(), Value::Null).unwrap();
        let effect = EffectInstance::new(
            id,
            definition,
            PreparedCausalRef::new(RequestUid::from_bytes([4; 16]), None, 0).unwrap(),
            target,
            turn(10),
            EffectDuration::Until { expires: turn(12) },
            1,
            state,
        )
        .unwrap();
        assert!(!effect.is_expired_at(&turn(11)).unwrap());
        assert!(effect.is_expired_at(&turn(12)).unwrap());
        assert!(
            effect
                .is_expired_at(&ClockStamp {
                    clock: ClockRef::SceneTurn {
                        scene: 9,
                        generation: 3,
                    },
                    at: 12,
                })
                .is_err()
        );
    }
}
