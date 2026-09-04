//! Rules-neutral opaque contact identity and lifecycle.
//!
//! A contact is private authoritative state owned by the projecting module.
//! Its public [`ContactId`] never contains the underlying [`SubjectKey`]. The
//! kernel enforces bounded audiences, monotonic generations/revisions, exact
//! typed observation clocks, terminal loss, and registered witness issuance;
//! a tactics/rules provider decides how observation and sharing actually work.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::action::{
    ActionError, ActionErrorCode, AuthorityGuard, AuthorityGuardContext, AuthorityGuardValidator,
    AuthorityWitness, AuthorityWitnessRequest, ContactId, PrincipalId, SubjectKey,
};
use crate::action_state::{
    StateFamilyDescriptor, StateFamilyDescriptorError, StateFamilyHandle, StateFamilyId,
    StateKeyCodec, StateValueDecoder, StateValueSerializer, StateValueValidator,
};
use crate::clock::{ClockMismatch, ClockStamp};
use crate::mechanics::{CanonicalEncodingError, CanonicalId, canonical_json_bytes_with_limit};

/// Maximum principals that may receive one shared contact identity.
pub const MAX_CONTACT_AUDIENCE: usize = 32;
/// Maximum canonical bytes in one contact record.
pub const MAX_CONTACT_BYTES: usize = 64 * 1024;

/// Whether an opaque contact is currently resolvable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContactLifecycle {
    Active,
    Lost {
        lost_at: ClockStamp,
        reason: CanonicalId,
    },
}

/// Construction surface for one newly observed contact.
#[derive(Clone, Debug)]
pub struct ContactStateDraft {
    pub truth_subject: SubjectKey,
    pub audience: Vec<PrincipalId>,
    pub observed_at: ClockStamp,
}

/// Durable private mapping from an opaque family key to authoritative truth.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContactState {
    schema_version: u16,
    truth_subject: SubjectKey,
    audience: Vec<PrincipalId>,
    generation: u64,
    revision: u64,
    observed_at: ClockStamp,
    lifecycle: ContactLifecycle,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContactStateWire {
    schema_version: u16,
    truth_subject: SubjectKey,
    audience: Vec<PrincipalId>,
    generation: u64,
    revision: u64,
    observed_at: ClockStamp,
    lifecycle: ContactLifecycle,
}

impl<'de> Deserialize<'de> for ContactState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ContactStateWire::deserialize(deserializer)?;
        Self::from_parts(wire).map_err(serde::de::Error::custom)
    }
}

impl ContactState {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn new(mut draft: ContactStateDraft) -> Result<Self, ContactStateError> {
        draft.audience.sort();
        Self::from_parts(ContactStateWire {
            schema_version: Self::SCHEMA_VERSION,
            truth_subject: draft.truth_subject,
            audience: draft.audience,
            generation: 1,
            revision: 1,
            observed_at: draft.observed_at,
            lifecycle: ContactLifecycle::Active,
        })
    }

    fn from_parts(wire: ContactStateWire) -> Result<Self, ContactStateError> {
        let state = Self {
            schema_version: wire.schema_version,
            truth_subject: wire.truth_subject,
            audience: wire.audience,
            generation: wire.generation,
            revision: wire.revision,
            observed_at: wire.observed_at,
            lifecycle: wire.lifecycle,
        };
        state.validate()?;
        Ok(state)
    }

    #[must_use]
    pub const fn truth_subject(&self) -> &SubjectKey {
        &self.truth_subject
    }

    #[must_use]
    pub fn audience(&self) -> &[PrincipalId] {
        &self.audience
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn observed_at(&self) -> &ClockStamp {
        &self.observed_at
    }

    #[must_use]
    pub const fn lifecycle(&self) -> &ContactLifecycle {
        &self.lifecycle
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self.lifecycle, ContactLifecycle::Active)
    }

    #[must_use]
    pub fn is_visible_to(&self, principal: &PrincipalId) -> bool {
        self.is_active() && self.audience.binary_search(principal).is_ok()
    }

    /// Refresh an active observation without changing its identity generation.
    pub fn checked_refresh(
        &self,
        expected_revision: u64,
        observed_at: ClockStamp,
    ) -> Result<Self, ContactStateError> {
        self.require_revision(expected_revision)?;
        self.require_active()?;
        match observed_at.checked_cmp(&self.observed_at)? {
            Ordering::Less => return Err(ContactStateError::ObservationRegressed),
            Ordering::Equal | Ordering::Greater => {}
        }
        Self {
            revision: self.next_revision()?,
            observed_at,
            ..self.clone()
        }
        .checked_post_image()
    }

    /// Share the same opaque contact with one additional explicit principal.
    /// Existing references remain valid because the truth mapping did not
    /// change; only the record revision advances.
    pub fn checked_share(
        &self,
        expected_revision: u64,
        principal: PrincipalId,
    ) -> Result<Self, ContactStateError> {
        self.require_revision(expected_revision)?;
        self.require_active()?;
        let mut audience = self.audience.clone();
        match audience.binary_search(&principal) {
            Ok(_) => return Err(ContactStateError::AlreadyShared),
            Err(index) => audience.insert(index, principal),
        }
        if audience.len() > MAX_CONTACT_AUDIENCE {
            return Err(ContactStateError::AudienceLimit {
                actual: audience.len(),
                max: MAX_CONTACT_AUDIENCE,
            });
        }
        Self {
            audience,
            revision: self.next_revision()?,
            ..self.clone()
        }
        .checked_post_image()
    }

    /// Revoke one recipient without changing references held by the remaining
    /// audience. Losing the last recipient must use [`Self::checked_mark_lost`]
    /// so terminal state keeps an explicit witness.
    pub fn checked_unshare(
        &self,
        expected_revision: u64,
        principal: &PrincipalId,
    ) -> Result<Self, ContactStateError> {
        self.require_revision(expected_revision)?;
        self.require_active()?;
        let mut audience = self.audience.clone();
        let index = audience
            .binary_search(principal)
            .map_err(|_| ContactStateError::NotShared)?;
        if audience.len() == 1 {
            return Err(ContactStateError::LastAudience);
        }
        audience.remove(index);
        Self {
            audience,
            revision: self.next_revision()?,
            ..self.clone()
        }
        .checked_post_image()
    }

    /// Invalidate every issued reference while preserving the private mapping
    /// for audit/recovery. Re-observation may reuse the ID only through
    /// [`Self::checked_reacquire`], which advances generation again.
    pub fn checked_mark_lost(
        &self,
        expected_revision: u64,
        lost_at: ClockStamp,
        reason: CanonicalId,
    ) -> Result<Self, ContactStateError> {
        self.require_revision(expected_revision)?;
        self.require_active()?;
        require_not_before(&lost_at, &self.observed_at)?;
        Self {
            generation: self.next_generation()?,
            revision: self.next_revision()?,
            lifecycle: ContactLifecycle::Lost { lost_at, reason },
            ..self.clone()
        }
        .checked_post_image()
    }

    /// Re-activate one preserved mapping after a fresh observation. The same
    /// `ContactId` is safe because every older reference has a lower generation.
    pub fn checked_reacquire(
        &self,
        expected_revision: u64,
        observed_at: ClockStamp,
    ) -> Result<Self, ContactStateError> {
        self.require_revision(expected_revision)?;
        if self.is_active() {
            return Err(ContactStateError::AlreadyActive);
        }
        Self {
            generation: self.next_generation()?,
            revision: self.next_revision()?,
            observed_at,
            lifecycle: ContactLifecycle::Active,
            ..self.clone()
        }
        .checked_post_image()
    }

    #[must_use]
    pub fn matches_reference(&self, principal: &PrincipalId, generation: u64) -> bool {
        self.generation == generation && self.is_visible_to(principal)
    }

    pub fn validate(&self) -> Result<(), ContactStateError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(ContactStateError::UnsupportedSchema {
                actual: self.schema_version,
                supported: Self::SCHEMA_VERSION,
            });
        }
        if self.generation == 0 || self.revision == 0 {
            return Err(ContactStateError::ZeroCounter);
        }
        if self.revision < self.generation {
            return Err(ContactStateError::RevisionBehindGeneration {
                revision: self.revision,
                generation: self.generation,
            });
        }
        if self.audience.is_empty() || self.audience.len() > MAX_CONTACT_AUDIENCE {
            return Err(ContactStateError::AudienceLimit {
                actual: self.audience.len(),
                max: MAX_CONTACT_AUDIENCE,
            });
        }
        if self.audience.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(ContactStateError::NonCanonicalAudience);
        }
        if let ContactLifecycle::Lost { lost_at, .. } = &self.lifecycle {
            require_not_before(lost_at, &self.observed_at)?;
        }
        canonical_json_bytes_with_limit(&json!(self), MAX_CONTACT_BYTES)
            .map_err(ContactStateError::Encoding)?;
        Ok(())
    }

    fn require_revision(&self, expected: u64) -> Result<(), ContactStateError> {
        if self.revision != expected {
            return Err(ContactStateError::StaleRevision {
                expected,
                actual: self.revision,
            });
        }
        Ok(())
    }

    fn require_active(&self) -> Result<(), ContactStateError> {
        if self.is_active() {
            Ok(())
        } else {
            Err(ContactStateError::NotActive)
        }
    }

    fn next_generation(&self) -> Result<u64, ContactStateError> {
        self.generation
            .checked_add(1)
            .ok_or(ContactStateError::GenerationExhausted)
    }

    fn next_revision(&self) -> Result<u64, ContactStateError> {
        self.revision
            .checked_add(1)
            .ok_or(ContactStateError::RevisionExhausted)
    }

    fn checked_post_image(self) -> Result<Self, ContactStateError> {
        self.validate()?;
        Ok(self)
    }
}

fn require_not_before(actual: &ClockStamp, floor: &ClockStamp) -> Result<(), ContactStateError> {
    if actual.checked_cmp(floor)? == Ordering::Less {
        return Err(ContactStateError::ObservationRegressed);
    }
    Ok(())
}

/// Invalid private contact state or checked lifecycle transition.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ContactStateError {
    #[error("unsupported contact schema {actual}; supported schema is {supported}")]
    UnsupportedSchema { actual: u16, supported: u16 },
    #[error("contact generation and revision must both be nonzero")]
    ZeroCounter,
    #[error("contact revision {revision} is behind generation {generation}")]
    RevisionBehindGeneration { revision: u64, generation: u64 },
    #[error("contact audience size {actual} is outside 1..={max}")]
    AudienceLimit { actual: usize, max: usize },
    #[error("contact audience must be strictly sorted and duplicate-free")]
    NonCanonicalAudience,
    #[error("contact revision is stale: expected {expected}, actual {actual}")]
    StaleRevision { expected: u64, actual: u64 },
    #[error("contact is not active")]
    NotActive,
    #[error("contact is already active")]
    AlreadyActive,
    #[error("contact is already shared with that principal")]
    AlreadyShared,
    #[error("contact is not shared with that principal")]
    NotShared,
    #[error("the last contact audience member must be removed by marking the contact lost")]
    LastAudience,
    #[error("contact observation time moved backwards")]
    ObservationRegressed,
    #[error("contact generation is exhausted")]
    GenerationExhausted,
    #[error("contact revision is exhausted")]
    RevisionExhausted,
    #[error(transparent)]
    Clock(#[from] ClockMismatch),
    #[error("contact cannot be canonically encoded: {0}")]
    Encoding(CanonicalEncodingError),
}

/// Shared codec for one provider-owned authoritative contact family.
#[derive(Clone, Copy, Debug, Default)]
pub struct ContactStateCodec;

impl StateKeyCodec<ContactId> for ContactStateCodec {
    fn encode_key(&self, key: &ContactId) -> Result<String, String> {
        Ok(key.to_hex())
    }
}

impl StateValueDecoder<ContactState> for ContactStateCodec {
    fn decode_value(&self, value: &serde_json::Value) -> Result<ContactState, String> {
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())
    }
}

impl StateValueValidator<ContactState> for ContactStateCodec {
    fn validate_value(&self, value: &ContactState) -> Result<(), String> {
        value.validate().map_err(|error| error.to_string())
    }
}

impl StateValueSerializer<ContactState> for ContactStateCodec {
    fn serialize_value(&self, value: &ContactState) -> Result<serde_json::Value, String> {
        serde_json::to_value(value).map_err(|error| error.to_string())
    }
}

/// Build the reusable typed-family descriptor for opaque contacts.
pub fn contact_state_family_descriptor(
    family: StateFamilyId,
) -> Result<StateFamilyDescriptor, StateFamilyDescriptorError> {
    StateFamilyDescriptor::new(
        family,
        CanonicalId::new("contact_id.v1").expect("static canonical id"),
        CanonicalId::new("contact_state.v1").expect("static canonical id"),
        ContactState::SCHEMA_VERSION,
        CanonicalId::new("contact_decode.v1").expect("static canonical id"),
        CanonicalId::new("contact_validate.v1").expect("static canonical id"),
        CanonicalId::new("contact_serialize.v1").expect("static canonical id"),
    )?
    .with_limits(64, MAX_CONTACT_BYTES)
}

/// Typed-family authority provider for opaque contact resolution and final
/// stale validation.
#[derive(Clone)]
pub struct ContactGuardValidator {
    contacts: StateFamilyHandle<ContactId, ContactState>,
}

impl fmt::Debug for ContactGuardValidator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContactGuardValidator")
            .field("owner", self.contacts.owner())
            .field("family", self.contacts.descriptor().id())
            .finish()
    }
}

impl ContactGuardValidator {
    #[must_use]
    pub const fn new(contacts: StateFamilyHandle<ContactId, ContactState>) -> Self {
        Self { contacts }
    }

    fn read<'a>(
        &self,
        context: &'a AuthorityGuardContext<'_>,
        id: &ContactId,
    ) -> Result<Option<&'a ContactState>, ActionError> {
        context.action_state(&self.contacts, id).map_err(|_| {
            ActionError::new(
                ActionErrorCode::Integrity,
                false,
                "contact authority state is unavailable",
            )
            .expect("static contact read error is bounded")
        })
    }
}

impl AuthorityGuardValidator for ContactGuardValidator {
    fn is_current(
        &self,
        context: &AuthorityGuardContext<'_>,
        guard: &AuthorityGuard,
    ) -> Result<bool, ActionError> {
        let AuthorityGuard::Contact {
            provider,
            principal,
            id,
            generation,
        } = guard
        else {
            return Ok(false);
        };
        if provider != self.contacts.owner() {
            return Ok(false);
        }
        Ok(self
            .read(context, id)?
            .is_some_and(|contact| contact.matches_reference(principal, *generation)))
    }

    fn witness(
        &self,
        context: &AuthorityGuardContext<'_>,
        request: &AuthorityWitnessRequest,
    ) -> Result<Option<AuthorityWitness>, ActionError> {
        let AuthorityWitnessRequest::Contact {
            provider,
            principal,
            id,
            generation,
        } = request
        else {
            return Ok(None);
        };
        if provider != self.contacts.owner() {
            return Ok(None);
        }
        let Some(contact) = self.read(context, id)? else {
            return Ok(None);
        };
        if !contact.matches_reference(principal, *generation) {
            return Ok(None);
        }
        Ok(Some(AuthorityWitness::Contact {
            truth_subject: contact.truth_subject().clone(),
            guard: AuthorityGuard::Contact {
                provider: provider.clone(),
                principal: principal.clone(),
                id: *id,
                generation: *generation,
            },
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use crate::action_state::{ActionStateStore, StateFamilyRegistryBuilder};
    use crate::clock::ClockRef;
    use crate::mechanics::{MechanicalOwnerReadiness, OwnerId};
    use crate::obj::ObjId;
    use crate::world::World;

    fn principal(id: u32) -> PrincipalId {
        PrincipalId::Object { id: ObjId(id) }
    }

    fn stamp(at: i64, generation: u64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::DomainTick {
                owner: OwnerId::new("tactics").unwrap(),
                id: CanonicalId::new("field").unwrap(),
                generation,
            },
            at,
        }
    }

    fn contact() -> ContactState {
        ContactState::new(ContactStateDraft {
            truth_subject: SubjectKey::Object { id: ObjId(90) },
            audience: vec![principal(2), principal(1)],
            observed_at: stamp(10, 4),
        })
        .unwrap()
    }

    #[test]
    fn sharing_is_canonical_and_revocation_is_per_principal() {
        let initial = contact();
        assert_eq!(initial.audience(), &[principal(1), principal(2)]);
        let shared = initial.checked_share(1, principal(3)).unwrap();
        assert_eq!(shared.generation(), 1);
        assert_eq!(shared.revision(), 2);
        assert!(shared.matches_reference(&principal(3), 1));
        let revoked = shared.checked_unshare(2, &principal(2)).unwrap();
        assert!(!revoked.matches_reference(&principal(2), 1));
        assert!(revoked.matches_reference(&principal(1), 1));
    }

    #[test]
    fn loss_and_reacquisition_invalidate_every_old_reference() {
        let initial = contact();
        let lost = initial
            .checked_mark_lost(1, stamp(12, 4), CanonicalId::new("occluded").unwrap())
            .unwrap();
        assert_eq!(lost.generation(), 2);
        assert!(!lost.matches_reference(&principal(1), 1));
        let reacquired = lost.checked_reacquire(2, stamp(13, 5)).unwrap();
        assert_eq!(reacquired.generation(), 3);
        assert_eq!(reacquired.revision(), 3);
        assert!(reacquired.matches_reference(&principal(1), 3));
        assert!(!reacquired.matches_reference(&principal(1), 1));
    }

    #[test]
    fn refresh_and_loss_require_a_comparable_nonregressing_clock() {
        let initial = contact();
        assert_eq!(
            initial.checked_refresh(1, stamp(9, 4)).unwrap_err(),
            ContactStateError::ObservationRegressed
        );
        assert!(matches!(
            initial.checked_refresh(1, stamp(11, 5)),
            Err(ContactStateError::Clock(_))
        ));
    }

    #[test]
    fn registered_provider_issues_private_truth_and_revalidates_it() {
        let owner = OwnerId::new("tactics").unwrap();
        let owners = BTreeSet::from([owner.clone()]);
        let readiness = MechanicalOwnerReadiness::for_owners(&owners);
        let mut families = StateFamilyRegistryBuilder::new();
        let codec = ContactStateCodec;
        let handle = families
            .register(
                owner.clone(),
                contact_state_family_descriptor(StateFamilyId::new("contacts").unwrap()).unwrap(),
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
        let id = ContactId::from_bytes([0x31; 16]);
        let state = contact();
        let mut transaction = store.transaction(&handle).unwrap();
        transaction.insert(&handle, &id, state.clone()).unwrap();
        let (_, installation) = transaction.prepare().unwrap().into_parts();
        store.install(installation);

        let world = World::new_minimal();
        let context = AuthorityGuardContext::new(&world, &store);
        let provider = ContactGuardValidator::new(handle);
        let request = AuthorityWitnessRequest::Contact {
            provider: owner,
            principal: principal(1),
            id,
            generation: state.generation(),
        };
        let witness = provider.witness(&context, &request).unwrap().unwrap();
        assert_eq!(witness.truth_subject(), Some(state.truth_subject()));
        assert!(provider.is_current(&context, witness.guard()).unwrap());
    }

    #[test]
    fn strict_decode_rejects_unknown_fields_and_noncanonical_audience() {
        let value = serde_json::to_value(contact()).unwrap();
        let mut reversed = value.clone();
        reversed["audience"].as_array_mut().unwrap().reverse();
        assert!(serde_json::from_value::<ContactState>(reversed).is_err());
        let mut unknown = value;
        unknown["truth_object"] = json!(90);
        assert!(serde_json::from_value::<ContactState>(unknown).is_err());
    }
}
