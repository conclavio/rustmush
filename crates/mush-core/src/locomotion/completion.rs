//! Composition-bound coverage of actual movement observers. Inspection and
//! retained snapshots grant no execution or movement authority. A registered
//! adapter may refuse a particular route; that never authorizes skipping it.

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::mechanics::{
    DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionRegistry, DefinitionValidationContext, OwnerId,
};
use crate::moved_observer::{MAX_MOVED_OBSERVERS, MovedObserverRoster};
use crate::participant::{OwnerParticipant, ParticipantRegistry};
use crate::{ModError, canonical_json_bytes_with_limit};

pub const MAX_MOVEMENT_COMPLETION_BINDINGS: usize = 16;
pub const MAX_MOVEMENT_MANIFEST_BYTES: usize = 64 * 1024;
const KIND: &str = "moved_completion";

fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
pub(crate) fn kind() -> Result<DefinitionKind, ModError> {
    DefinitionKind::new(KIND).map_err(bad)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    format: u16,
    action: DefinitionRef,
    participant: DefinitionRef,
    observers: Vec<DefinitionRef>,
}
impl Binding {
    fn validate(&self, owner: &OwnerId) -> Result<(), ModError> {
        if self.format != 1
            || self.action.key().kind.as_str() != "action"
            || self.participant.key().kind.as_str() != "owner_participant"
            || &self.participant.key().owner != owner
            || self.observers.is_empty()
            || self.observers.len() > MAX_MOVED_OBSERVERS
            || self.observers.iter().any(|observer| {
                &observer.key().owner != owner || observer.key().kind.as_str() != "moved_observer"
            })
            || self.observers.iter().collect::<BTreeSet<_>>().len() != self.observers.len()
        {
            return Err(bad("invalid movement completion binding"));
        }
        Ok(())
    }
    fn references(&self) -> Vec<DefinitionRef> {
        std::iter::once(&self.action)
            .chain(std::iter::once(&self.participant))
            .chain(&self.observers)
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

/// Metadata alone does not register an executable completion adapter.
pub fn binding_descriptor(
    id: &str,
    version: u32,
    action: DefinitionRef,
    participant: DefinitionRef,
    observers: Vec<DefinitionRef>,
) -> Result<DefinitionDescriptor, ModError> {
    let body = Binding {
        format: 1,
        action,
        participant,
        observers,
    };
    body.validate(&body.participant.key().owner)?;
    DefinitionDescriptor::from_serializable(
        kind()?,
        DefinitionId::new(id).map_err(bad)?,
        version,
        &body,
    )
    .map(|descriptor| descriptor.with_references(body.references()))
    .map_err(bad)
}

pub(crate) fn validate_definition(ctx: &DefinitionValidationContext<'_>) -> Result<(), String> {
    let entry = ctx.definition();
    let body: Binding = serde_json::from_value(entry.payload().clone())
        .map_err(|_| "movement completion binding cannot decode".to_string())?;
    body.validate(&entry.reference().key().owner)
        .map_err(|e| e.to_string())?;
    if entry.reference().key().kind.as_str() != KIND || entry.references() != body.references() {
        return Err("movement completion binding references differ".into());
    }
    let expected = crate::DefinitionRegistryBuilder::new()
        .register(
            entry.reference().key().owner.clone(),
            binding_descriptor(
                entry.reference().key().id.as_str(),
                entry.reference().version(),
                body.action,
                body.participant,
                body.observers,
            )
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    if entry.reference() != &expected {
        return Err("movement completion binding identity differs".into());
    }
    Ok(())
}

/// Registration coverage, not successful preparation of any particular move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MovementCoverage {
    LegacyBlocker,
    Prepared { group: u16 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovementManifestEntry {
    ordinal: u16,
    owner: OwnerId,
    #[serde(deserialize_with = "required_option")]
    observer: Option<DefinitionRef>,
    coverage: MovementCoverage,
}
impl MovementManifestEntry {
    pub const fn ordinal(&self) -> u16 {
        self.ordinal
    }
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }
    pub const fn observer(&self) -> Option<&DefinitionRef> {
        self.observer.as_ref()
    }
    pub const fn coverage(&self) -> MovementCoverage {
        self.coverage
    }
}

/// One actual owner slot may cover several contiguous callbacks. Bindings for
/// interleaved callbacks require a richer ordering protocol and refuse here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovementManifestGroup {
    owner: OwnerId,
    binding: DefinitionRef,
    participant: DefinitionRef,
    ordinals: Vec<u16>,
}
impl MovementManifestGroup {
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }
    pub const fn binding(&self) -> &DefinitionRef {
        &self.binding
    }
    pub const fn participant(&self) -> &DefinitionRef {
        &self.participant
    }
    pub fn ordinals(&self) -> &[u16] {
        &self.ordinals
    }
}

/// Retainable, strictly decoded metadata. A decoded snapshot is not a frozen
/// composition or an executable capability. Its historical references are
/// checked against retained definitions, never against today's callback roster.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MovementManifestSnapshot {
    format: u16,
    action: DefinitionRef,
    roster_digest: DefinitionDigest,
    entries: Vec<MovementManifestEntry>,
    groups: Vec<MovementManifestGroup>,
}
impl MovementManifestSnapshot {
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn roster_digest(&self) -> DefinitionDigest {
        self.roster_digest
    }
    pub fn entries(&self) -> &[MovementManifestEntry] {
        &self.entries
    }
    pub fn groups(&self) -> &[MovementManifestGroup] {
        &self.groups
    }
    pub fn has_blockers(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.coverage == MovementCoverage::LegacyBlocker)
    }
    pub fn encode(&self) -> Result<Vec<u8>, ModError> {
        let value = serde_json::to_value(self).map_err(bad)?;
        canonical_json_bytes_with_limit(&value, MAX_MOVEMENT_MANIFEST_BYTES).map_err(bad)
    }
    pub fn digest(&self) -> Result<DefinitionDigest, ModError> {
        DefinitionDigest::for_canonical_value_with_limit(
            "core.movement_completion_manifest.v1",
            &serde_json::to_value(self).map_err(bad)?,
            MAX_MOVEMENT_MANIFEST_BYTES,
        )
        .map_err(bad)
    }
    pub fn decode(bytes: &[u8], definitions: &DefinitionRegistry) -> Result<Self, ModError> {
        if bytes.len() > MAX_MOVEMENT_MANIFEST_BYTES {
            return Err(bad("movement manifest exceeds its byte bound"));
        }
        let wire: SnapshotWire = serde_json::from_slice(bytes).map_err(bad)?;
        let snapshot = Self {
            format: wire.format,
            action: wire.action,
            roster_digest: wire.roster_digest,
            entries: wire.entries,
            groups: wire.groups,
        };
        snapshot.validate(definitions)?;
        if snapshot.encode()? != bytes {
            return Err(bad("movement manifest is not canonical"));
        }
        Ok(snapshot)
    }
    pub(crate) fn from_value(value: &serde_json::Value) -> Result<Self, ModError> {
        canonical_json_bytes_with_limit(value, MAX_MOVEMENT_MANIFEST_BYTES).map_err(bad)?;
        let wire: SnapshotWire = serde_json::from_value(value.clone()).map_err(bad)?;
        let snapshot = Self {
            format: wire.format,
            action: wire.action,
            roster_digest: wire.roster_digest,
            entries: wire.entries,
            groups: wire.groups,
        };
        snapshot.validate_with(None)?;
        if serde_json::to_value(&snapshot).map_err(bad)? != *value {
            return Err(bad("movement manifest value is not canonical"));
        }
        Ok(snapshot)
    }
    fn validate(&self, definitions: &DefinitionRegistry) -> Result<(), ModError> {
        self.validate_with(Some(definitions))
    }
    fn validate_with(&self, definitions: Option<&DefinitionRegistry>) -> Result<(), ModError> {
        if self.format != 1
            || self.action.key().kind.as_str() != "action"
            || self.entries.len() > MAX_MOVED_OBSERVERS
            || self.groups.len() > MAX_MOVEMENT_COMPLETION_BINDINGS
        {
            return Err(bad("movement manifest shape differs"));
        }
        if let Some(definitions) = definitions {
            definitions.inspect_exact(&self.action).map_err(bad)?;
        }
        let mut roster = Vec::with_capacity(self.entries.len());
        let mut named_observers = BTreeSet::new();
        for (ordinal, entry) in self.entries.iter().enumerate() {
            if usize::from(entry.ordinal) != ordinal {
                return Err(bad("movement manifest callback order differs"));
            }
            if let Some(observer) = &entry.observer {
                if !named_observers.insert(observer) {
                    return Err(bad("movement manifest repeats a named callback"));
                }
                let expected = crate::DefinitionRegistryBuilder::new()
                    .register(
                        entry.owner.clone(),
                        crate::moved_observer::descriptor(
                            observer.key().id.as_str(),
                            observer.version(),
                        )?,
                    )
                    .map_err(bad)?;
                if observer.key().owner != entry.owner
                    || observer.key().kind.as_str() != "moved_observer"
                    || observer != &expected
                {
                    return Err(bad("movement manifest observer identity differs"));
                }
                if let Some(definitions) = definitions {
                    definitions.inspect_exact(observer).map_err(bad)?;
                }
            } else if entry.coverage != MovementCoverage::LegacyBlocker {
                return Err(bad(
                    "anonymous movement callback cannot have prepared coverage",
                ));
            }
            roster.push(serde_json::json!({
                "owner":entry.owner,"definition":entry.observer,"mode":"legacy_post_mutation"
            }));
        }
        let roster_digest = DefinitionDigest::for_canonical_value_with_limit(
            "core.moved_observer_roster.v1",
            &serde_json::to_value(roster).map_err(bad)?,
            16 * 1024,
        )
        .map_err(bad)?;
        if roster_digest != self.roster_digest {
            return Err(bad("movement manifest roster digest differs"));
        }
        let mut previous = None;
        let mut used_participants = BTreeSet::new();
        let mut used_bindings = BTreeSet::new();
        for (index, group) in self.groups.iter().enumerate() {
            let first = *group
                .ordinals
                .first()
                .ok_or_else(|| bad("empty movement group"))?;
            if group.ordinals.len() > MAX_MOVED_OBSERVERS
                || previous.is_some_and(|last| first <= last)
                || group
                    .ordinals
                    .windows(2)
                    .any(|pair| pair[0].checked_add(1) != Some(pair[1]))
                || group.binding.key().owner != group.owner
                || group.binding.key().kind.as_str() != KIND
                || group.participant.key().owner != group.owner
                || group.participant.key().kind.as_str() != "owner_participant"
                || !used_participants.insert(&group.participant)
                || !used_bindings.insert(&group.binding)
            {
                return Err(bad("movement manifest owner group differs"));
            }
            previous = group.ordinals.last().copied();
            let mut observers = Vec::with_capacity(group.ordinals.len());
            for ordinal in &group.ordinals {
                let entry = self
                    .entries
                    .get(usize::from(*ordinal))
                    .ok_or_else(|| bad("movement group callback is absent"))?;
                if entry.owner != group.owner
                    || entry.coverage
                        != (MovementCoverage::Prepared {
                            group: u16::try_from(index).map_err(bad)?,
                        })
                {
                    return Err(bad("movement group callback coverage differs"));
                }
                observers.push(
                    entry
                        .observer
                        .clone()
                        .ok_or_else(|| bad("anonymous prepared callback"))?,
                );
            }
            let expected = crate::DefinitionRegistryBuilder::new()
                .register(
                    group.owner.clone(),
                    binding_descriptor(
                        group.binding.key().id.as_str(),
                        group.binding.version(),
                        self.action.clone(),
                        group.participant.clone(),
                        observers,
                    )?,
                )
                .map_err(bad)?;
            if group.binding != expected {
                return Err(bad("movement group differs from its retained binding"));
            }
            if let Some(definitions) = definitions {
                definitions.inspect_exact(&group.binding).map_err(bad)?;
                definitions.inspect_exact(&group.participant).map_err(bad)?;
            }
        }
        for entry in &self.entries {
            if let MovementCoverage::Prepared { group } = entry.coverage
                && !self
                    .groups
                    .get(usize::from(group))
                    .is_some_and(|group| group.ordinals.contains(&entry.ordinal))
            {
                return Err(bad("movement callback has no prepared group"));
            }
        }
        Ok(())
    }
}

/// Constructed only by the actual mechanics freeze after adapter/grant/slot
/// resolution. Its read-only snapshot never authorizes a route or installation.
pub struct FrozenMovementManifest {
    snapshot: MovementManifestSnapshot,
    digest: DefinitionDigest,
}

/// A closed service receives its snapshot only from actual composition freeze.
/// Legacy observer mutation invalidates every clone monotonically.
#[derive(Clone)]
pub(crate) struct MovementManifestLease {
    snapshot: Arc<OnceLock<MovementManifestSnapshot>>,
    invalidated: Arc<AtomicBool>,
}
impl MovementManifestLease {
    pub(crate) fn snapshot(
        &self,
    ) -> Result<&MovementManifestSnapshot, crate::participant::ParticipantError> {
        if self.invalidated.load(Ordering::Acquire) {
            return Err(crate::participant::ParticipantError::Authority);
        }
        self.snapshot
            .get()
            .ok_or(crate::participant::ParticipantError::Unavailable)
    }
    pub(crate) fn check(&self) -> Result<(), crate::participant::ParticipantError> {
        self.snapshot().map(|_| ())
    }
}

struct ScopeRequest {
    action: DefinitionRef,
    participants: BTreeSet<DefinitionRef>,
    lease: MovementManifestLease,
}
impl FrozenMovementManifest {
    pub const fn snapshot(&self) -> &MovementManifestSnapshot {
        &self.snapshot
    }
    pub const fn digest(&self) -> DefinitionDigest {
        self.digest
    }
}

struct Registration {
    adapter_type: TypeId,
    owner: OwnerId,
    binding: DefinitionRef,
    body: Binding,
}
struct Requirement {
    adapter_type: TypeId,
    action: DefinitionRef,
    participant: DefinitionRef,
    observers: Vec<DefinitionRef>,
}
#[derive(Default)]
pub(crate) struct MovementCompletionBuilder {
    registrations: Vec<Registration>,
    requirements: Vec<Requirement>,
    scopes: Vec<ScopeRequest>,
}
impl MovementCompletionBuilder {
    pub(crate) fn is_empty(&self) -> bool {
        self.registrations.is_empty() && self.requirements.is_empty() && self.scopes.is_empty()
    }
    pub(crate) fn request_scope(
        &mut self,
        action: DefinitionRef,
        participants: Vec<DefinitionRef>,
        invalidated: Arc<AtomicBool>,
    ) -> Result<MovementManifestLease, ModError> {
        if self.scopes.len() >= MAX_MOVEMENT_COMPLETION_BINDINGS
            || participants.is_empty()
            || participants.len() > MAX_MOVEMENT_COMPLETION_BINDINGS
            || self.scopes.iter().any(|scope| scope.action == action)
        {
            return Err(bad(
                "movement manifest scope conflicts or exceeds its bound",
            ));
        }
        let unique: BTreeSet<_> = participants.iter().cloned().collect();
        if unique.len() != participants.len() {
            return Err(bad("movement manifest scope repeats an owner participant"));
        }
        let lease = MovementManifestLease {
            snapshot: Arc::new(OnceLock::new()),
            invalidated,
        };
        self.scopes.push(ScopeRequest {
            action,
            participants: unique,
            lease: lease.clone(),
        });
        Ok(lease)
    }
    pub(crate) fn register<P: OwnerParticipant>(
        &mut self,
        owner: OwnerId,
        binding: DefinitionRef,
        action: DefinitionRef,
        participant: DefinitionRef,
        observers: Vec<DefinitionRef>,
    ) -> Result<(), ModError> {
        let body = Binding {
            format: 1,
            action,
            participant,
            observers,
        };
        body.validate(&owner)?;
        if self.registrations.len() >= MAX_MOVEMENT_COMPLETION_BINDINGS
            || binding.key().owner != owner
            || binding.key().kind.as_str() != KIND
            || self.registrations.iter().any(|registered| {
                registered.binding == binding
                    || (registered.body.action == body.action
                        && (registered.adapter_type == TypeId::of::<P>()
                            || registered
                                .body
                                .observers
                                .iter()
                                .any(|observer| body.observers.contains(observer))))
            })
        {
            return Err(bad(
                "movement completion registration conflicts or exceeds its bound",
            ));
        }
        self.registrations.push(Registration {
            adapter_type: TypeId::of::<P>(),
            owner,
            binding,
            body,
        });
        Ok(())
    }
    pub(crate) fn require<P: OwnerParticipant>(
        &mut self,
        action: DefinitionRef,
        participant: DefinitionRef,
        observers: Vec<DefinitionRef>,
    ) -> Result<(), ModError> {
        if self.requirements.len() >= MAX_MOVEMENT_COMPLETION_BINDINGS
            || observers.is_empty()
            || observers.len() > MAX_MOVED_OBSERVERS
        {
            return Err(bad("movement completion requirements exceed their bound"));
        }
        self.requirements.push(Requirement {
            adapter_type: TypeId::of::<P>(),
            action,
            participant,
            observers,
        });
        Ok(())
    }
    pub(crate) fn freeze(
        self,
        roster: &MovedObserverRoster,
        definitions: &DefinitionRegistry,
        participants: &ParticipantRegistry,
    ) -> Result<BTreeMap<DefinitionRef, FrozenMovementManifest>, ModError> {
        for requirement in &self.requirements {
            if !self.registrations.iter().any(|registration| {
                registration.adapter_type == requirement.adapter_type
                    && registration.body.action == requirement.action
                    && registration.body.participant == requirement.participant
                    && registration.body.observers == requirement.observers
            }) {
                return Err(bad(
                    "closed movement protocol lacks its actual registered completion group",
                ));
            }
        }
        let mut by_action: BTreeMap<DefinitionRef, Vec<MovementManifestGroup>> = BTreeMap::new();
        for registration in self.registrations {
            let body = registration.body;
            if definitions.current(registration.binding.key()) != Some(&registration.binding)
                || definitions.current(body.action.key()) != Some(&body.action)
                || definitions.current(body.participant.key()) != Some(&body.participant)
                || !participants.grants_exact_type(
                    registration.adapter_type,
                    &body.participant,
                    &body.action.key().owner,
                    &body.action,
                )
            {
                return Err(bad(
                    "movement completion lacks the current actual adapter, request or grant",
                ));
            }
            definitions.get_exact(&registration.binding).map_err(bad)?;
            let mut ordinals = Vec::with_capacity(body.observers.len());
            for observer in &body.observers {
                let mut matching = roster.entries().iter().enumerate().filter(|(_, entry)| {
                    entry.definition() == Some(observer) && entry.owner() == &registration.owner
                });
                let (ordinal, _) = matching.next().ok_or_else(|| {
                    bad("movement completion observer is not an actual current callback")
                })?;
                if matching.next().is_some() {
                    return Err(bad("movement completion observer is ambiguous"));
                }
                ordinals.push(u16::try_from(ordinal).map_err(bad)?);
            }
            if ordinals
                .windows(2)
                .any(|pair| pair[0].checked_add(1) != Some(pair[1]))
            {
                return Err(bad(
                    "movement completion callbacks are not contiguous in dispatch order",
                ));
            }
            by_action
                .entry(body.action)
                .or_default()
                .push(MovementManifestGroup {
                    owner: registration.owner,
                    binding: registration.binding,
                    participant: body.participant,
                    ordinals,
                });
        }
        let mut frozen = BTreeMap::new();
        for (action, mut groups) in by_action {
            groups.sort_by_key(|group| group.ordinals.first().copied());
            let mut entries: Vec<_> = roster
                .entries()
                .iter()
                .enumerate()
                .map(|(ordinal, entry)| {
                    Ok(MovementManifestEntry {
                        ordinal: u16::try_from(ordinal).map_err(bad)?,
                        owner: entry.owner().clone(),
                        observer: entry.definition().cloned(),
                        coverage: MovementCoverage::LegacyBlocker,
                    })
                })
                .collect::<Result<_, ModError>>()?;
            for (index, group) in groups.iter().enumerate() {
                for ordinal in &group.ordinals {
                    let entry = entries
                        .get_mut(usize::from(*ordinal))
                        .ok_or_else(|| bad("movement callback absent"))?;
                    if entry.coverage != MovementCoverage::LegacyBlocker {
                        return Err(bad("movement callback has duplicate prepared coverage"));
                    }
                    entry.coverage = MovementCoverage::Prepared {
                        group: u16::try_from(index).map_err(bad)?,
                    };
                }
            }
            let snapshot = MovementManifestSnapshot {
                format: 1,
                action: action.clone(),
                roster_digest: roster.digest(),
                entries,
                groups,
            };
            snapshot.validate(definitions)?;
            let digest = snapshot.digest()?;
            frozen.insert(action, FrozenMovementManifest { snapshot, digest });
        }
        for scope in self.scopes {
            let manifest = frozen
                .get(&scope.action)
                .ok_or_else(|| bad("closed movement scope has no frozen manifest"))?;
            let actual: BTreeSet<_> = manifest
                .snapshot
                .groups
                .iter()
                .map(|group| group.participant.clone())
                .collect();
            if actual != scope.participants {
                return Err(bad(
                    "closed movement scope differs from actual prepared coverage",
                ));
            }
            scope
                .lease
                .snapshot
                .set(manifest.snapshot.clone())
                .map_err(|_| bad("movement manifest lease already belongs to a composition"))?;
        }
        Ok(frozen)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotWire {
    format: u16,
    action: DefinitionRef,
    roster_digest: DefinitionDigest,
    entries: Vec<MovementManifestEntry>,
    groups: Vec<MovementManifestGroup>,
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

#[cfg(test)]
#[path = "completion_tests.rs"]
mod tests;
