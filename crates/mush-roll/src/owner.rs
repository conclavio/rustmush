use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mush_core::{
    GameModule, ModError, ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, ObjId, Registrar,
};
use serde::de::DeserializeOwned;
use smol_str::SmolStr;

use crate::model::*;
use crate::{RollError, invalid};

pub const NS: &str = "roll";
const COLLECTION_QUARANTINE_PREFIX: &str = "collection:";
const MAX_COLLECTION_GUARDS: usize = 64;
const MAX_COLLECTION_REASON_BYTES: usize = 64;

#[derive(Clone, Debug)]
enum Change {
    Library(Library),
    Mind(Mind),
    Revision(Revision),
    Reference(ReferenceRecord),
    RemoveMind(ObjId, u64),
    RemoveRevision(RevisionKey),
    RemoveReference(ReferenceKey),
}

impl Change {
    fn operation(&self) -> Result<ModuleOp, RollError> {
        let pair = match self {
            Self::Library(value) => {
                value.validate()?;
                Some((
                    value.key(),
                    encoded(value, &value.key(), MAX_DOCUMENT_BYTES)?,
                ))
            }
            Self::Mind(value) => {
                value.validate()?;
                Some((
                    value.key(),
                    encoded(value, &value.key(), MAX_DOCUMENT_BYTES)?,
                ))
            }
            Self::Revision(value) => {
                value.validate()?;
                Some((
                    value.reference.key.key(),
                    encoded(value, &value.reference.key.key(), MAX_DOCUMENT_BYTES)?,
                ))
            }
            Self::Reference(value) => {
                value.validate()?;
                Some((
                    value.key.key(),
                    encoded(value, &value.key.key(), MAX_DOCUMENT_BYTES)?,
                ))
            }
            _ => None,
        };
        let op = match pair {
            Some((key, value)) => ModuleWrite::JsonSet {
                key: key.into(),
                value,
            },
            None => ModuleWrite::Del {
                key: match self {
                    Self::RemoveMind(character, id) => format!("mind:{}:{id}", character.0),
                    Self::RemoveRevision(key) => key.key(),
                    Self::RemoveReference(key) => key.key(),
                    _ => return Err(invalid("change", "unsupported removal")),
                }
                .into(),
            },
        };
        Ok(ModuleOp {
            ns: SmolStr::new_static(NS),
            op,
        })
    }
}

/// An opaque fully validated, serialized delta. All edits are conditional on
/// the store generation observed during preparation. It never contains source
/// owner state or executable callbacks.
pub struct PreparedChange {
    generation: u64,
    changes: Vec<Change>,
    operations: Vec<ModuleOp>,
}

impl PreparedChange {
    pub fn operations(&self) -> &[ModuleOp] {
        &self.operations
    }
}

/// One immutable binding between the validated roll delta, source-owner token
/// and revision identity. Only [`RollStore::prepare_save`] can construct it.
///
/// Callers cannot cross two preparations' source and revision records:
///
/// ```compile_fail
/// use mush_roll::PreparedSave;
/// fn cross<T>(left: &mut PreparedSave<T>, right: &mut PreparedSave<T>) {
///     std::mem::swap(&mut left.source, &mut right.source);
/// }
/// ```
///
/// Nor can they change the character that the combined publisher authenticates:
///
/// ```compile_fail
/// use mush_core::ObjId;
/// use mush_roll::PreparedSave;
/// fn impersonate<T>(save: &mut PreparedSave<T>) {
///     save.revision.key.character = ObjId(99);
/// }
/// ```
pub struct PreparedSave<T> {
    change: PreparedChange,
    source: PreparedSource<T>,
    revision: RevisionRef,
}

impl<T> PreparedSave<T> {
    pub fn revision(&self) -> RevisionRef {
        self.revision
    }

    pub fn source(&self) -> &PreparedSource<T> {
        &self.source
    }

    pub fn change(&self) -> &PreparedChange {
        &self.change
    }

    /// Consume the checked binding at the combined publisher's commit boundary.
    /// The publisher checks both owner tokens before applying either owner and
    /// queues these complete operations in one atomic world batch.
    pub fn into_parts(self) -> (PreparedChange, PreparedSource<T>, RevisionRef) {
        (self.change, self.source, self.revision)
    }
}

/// Read-only audit results prepared while both module state slots are shared.
/// Applying them later requires the same roll generation, so no revision can
/// enter between the audited snapshot and its publication barrier.
pub struct PreparedSourceAudit {
    generation: u64,
    failures: BTreeMap<String, String>,
}

/// Removed revisions' source hashes are only candidates for the source owner's
/// collector. That owner must also honor every other source user (including
/// stock content and older seals); this crate never deletes a source document.
pub struct PreparedPrune {
    pub change: PreparedChange,
    pub source_candidates: BTreeSet<mush_core::mechanics::DefinitionDigest>,
    pub removed: Vec<RevisionRef>,
}

#[derive(Clone, Debug, Default)]
pub struct RollStore {
    generation: u64,
    libraries: BTreeMap<ObjId, Library>,
    minds: BTreeMap<(ObjId, u64), Mind>,
    revisions: BTreeMap<RevisionKey, Revision>,
    references: BTreeMap<ReferenceKey, ReferenceRecord>,
    /// All occupied keys, including unreadable documents. They cannot be
    /// silently replaced by an operation that assumes absence.
    occupied: BTreeSet<String>,
    quarantine: BTreeMap<String, String>,
}

impl RollStore {
    pub fn quarantine(&self) -> &BTreeMap<String, String> {
        &self.quarantine
    }

    /// A retained-data owner could not reconstruct all of its revision roots.
    /// Add one boot-local, conservative collection guard for its exact canonical
    /// namespace. `reason` is a fixed identifier, never a record, actor or source
    /// detail. Recovery must derive this guard again on every boot before any
    /// collection is enabled; it is not a persisted second reference registry.
    ///
    /// Guards are additive and idempotent. There is deliberately no clear API:
    /// another owner cannot remove a guard, and a successful later partial read
    /// cannot establish that every unreadable retained record has been repaired.
    /// Rebuilding the store after complete recovery is the only removal path.
    /// Ordinary authoring and known retention records remain usable.
    pub fn quarantine_collection(&mut self, owner: &str, reason: &str) -> Result<(), RollError> {
        if owner.len() > mush_core::mechanics::CANONICAL_ID_MAX_BYTES {
            return Err(invalid(
                "collection owner",
                "expected an exact canonical namespace",
            ));
        }
        mush_core::mechanics::OwnerId::parse_canonical(owner)
            .map_err(|_| invalid("collection owner", "expected an exact canonical namespace"))?;
        if owner == NS {
            return Err(invalid("collection owner", "reserved retention owner"));
        }
        if reason.is_empty()
            || reason.len() > MAX_COLLECTION_REASON_BYTES
            || !reason
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
        {
            return Err(invalid(
                "collection reason",
                "expected 1..64 lowercase ASCII identifier bytes",
            ));
        }
        let key = format!("{COLLECTION_QUARANTINE_PREFIX}{owner}");
        if self.quarantine.contains_key(&key) {
            return Ok(());
        }
        if self
            .quarantine
            .keys()
            .filter(|key| key.starts_with(COLLECTION_QUARANTINE_PREFIX))
            .count()
            >= MAX_COLLECTION_GUARDS
        {
            return Err(RollError::Exhausted("collection guard"));
        }
        let next = self
            .generation
            .checked_add(1)
            .ok_or(RollError::Exhausted("store generation"))?;
        self.quarantine.insert(key, reason.into());
        self.generation = next;
        Ok(())
    }

    fn check_collection(&self) -> Result<(), RollError> {
        if self
            .quarantine
            .keys()
            .any(|key| key.starts_with(COLLECTION_QUARANTINE_PREFIX))
        {
            // Keep owner/reason diagnostics in the boot-local quarantine map;
            // public refusals must not expose another owner's retained data.
            return Err(RollError::Quarantined("external retention recovery".into()));
        }
        if let Some((key, _)) = self.quarantine.first_key_value() {
            return Err(RollError::Quarantined(key.clone()));
        }
        Ok(())
    }

    /// Actor identity is a character, never an account. Callers must pass the
    /// authenticated session's selected character, not a request-body owner.
    pub fn mind(&self, actor: ObjId, name: &str) -> Result<&Mind, RollError> {
        if self
            .quarantine
            .contains_key(&format!("library:{}", actor.0))
        {
            return Err(RollError::Missing);
        }
        let name = canonical_name(name)?;
        let id = self
            .libraries
            .get(&actor)
            .and_then(|l| l.names.get(&name))
            .ok_or(RollError::Missing)?;
        self.readable_mind(actor, *id)
    }

    /// Resolve an active owned mind by its stable library identity. Display-name
    /// changes do not change this identity; removal and quarantine refuse a new
    /// selection even when historical revisions remain retained.
    pub fn owned_mind(&self, actor: ObjId, id: u64) -> Result<&Mind, RollError> {
        self.readable_mind(actor, id)
    }

    /// All active library access, including a persisted Follow selection, must
    /// observe the same boot/source-audit quarantine as the named lookup.
    fn readable_mind(&self, actor: ObjId, id: u64) -> Result<&Mind, RollError> {
        if self
            .quarantine
            .contains_key(&format!("library:{}", actor.0))
        {
            return Err(RollError::Missing);
        }
        if !self
            .libraries
            .get(&actor)
            .is_some_and(|library| library.names.values().any(|held| *held == id))
        {
            return Err(RollError::Missing);
        }
        self.minds
            .get(&(actor, id))
            .filter(|mind| !self.quarantine.contains_key(&mind.key()))
            .ok_or(RollError::Missing)
    }

    pub fn names(&self, actor: ObjId) -> Vec<&str> {
        if self
            .quarantine
            .contains_key(&format!("library:{}", actor.0))
        {
            return Vec::new();
        }
        self.libraries
            .get(&actor)
            .map(|l| l.names.keys().map(String::as_str).collect())
            .unwrap_or_default()
    }

    pub fn revision(&self, actor: ObjId, key: RevisionKey) -> Result<&Revision, RollError> {
        if key.character != actor || self.quarantine.contains_key(&key.key()) {
            return Err(RollError::Missing);
        }
        self.revisions.get(&key).ok_or(RollError::Missing)
    }

    /// Only a complete digest is authoritative. This display convenience
    /// refuses prefixes matching distinct digests and is scoped to one owned
    /// mind. Repeated saves of one executable resolve to its latest local number.
    pub fn resolve_digest(
        &self,
        actor: ObjId,
        name: &str,
        prefix: &str,
    ) -> Result<RevisionRef, RollError> {
        let mind = self.mind(actor, name)?;
        if !(8..=64).contains(&prefix.len())
            || !prefix
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(invalid(
                "digest",
                "expected 8..64 lowercase hexadecimal digits",
            ));
        }
        let matches: BTreeMap<_, _> = self
            .revisions
            .values()
            .filter(|revision| {
                revision.reference.key.character == actor
                    && revision.reference.key.mind == mind.id
                    && revision.reference.digest.to_hex().starts_with(prefix)
                    && !self.quarantine.contains_key(&revision.reference.key.key())
            })
            .map(|revision| (revision.reference.digest, revision.reference))
            .collect();
        let matches: Vec<_> = matches.values().copied().collect();
        match matches.as_slice() {
            [one] => Ok(*one),
            [] => Err(RollError::Missing),
            _ => Err(RollError::Ambiguous),
        }
    }

    pub fn prepare_create(
        &self,
        actor: ObjId,
        name: &str,
        draft: WorkingCopy,
    ) -> Result<PreparedChange, RollError> {
        let name = canonical_name(name)?;
        draft.validate_bounds()?;
        let mut library = self
            .libraries
            .get(&actor)
            .cloned()
            .unwrap_or_else(|| Library::new(actor));
        if library.names.contains_key(&name) {
            return Err(RollError::Exists(name));
        }
        if library.names.len() >= MAX_MINDS {
            return Err(invalid("library", "maximum 20 authored minds"));
        }
        let id = library.next_mind;
        library.next_mind = id.checked_add(1).ok_or(RollError::Exhausted("mind"))?;
        library.names.insert(name.clone(), id);
        let mind = Mind {
            format: FORMAT,
            character: actor,
            id,
            name,
            edit: 0,
            next_revision: 1,
            head: None,
            draft,
            unnamed: VecDeque::new(),
            named: BTreeMap::new(),
        };
        if self.occupied.contains(&mind.key()) {
            return Err(RollError::Exists(mind.key()));
        }
        self.prepare(vec![Change::Library(library), Change::Mind(mind)])
    }

    pub fn prepare_edit(
        &self,
        actor: ObjId,
        name: &str,
        base: EditBase,
        draft: WorkingCopy,
    ) -> Result<PreparedChange, RollError> {
        draft.validate_bounds()?;
        let mut mind = self.editable(actor, name, base)?;
        mind.edit = mind
            .edit
            .checked_add(1)
            .ok_or(RollError::Exhausted("edit"))?;
        mind.draft = draft;
        self.prepare(vec![Change::Mind(mind)])
    }

    pub fn prepare_rename(
        &self,
        actor: ObjId,
        name: &str,
        base: EditBase,
        new_name: &str,
    ) -> Result<PreparedChange, RollError> {
        let mut mind = self.editable(actor, name, base)?;
        let new_name = canonical_name(new_name)?;
        let mut library = self
            .libraries
            .get(&actor)
            .cloned()
            .ok_or(RollError::Missing)?;
        if library.names.contains_key(&new_name) {
            return Err(RollError::Exists(new_name));
        }
        library.names.remove(&mind.name);
        library.names.insert(new_name.clone(), mind.id);
        mind.name = new_name;
        mind.edit = mind
            .edit
            .checked_add(1)
            .ok_or(RollError::Exhausted("edit"))?;
        self.prepare(vec![Change::Library(library), Change::Mind(mind)])
    }

    // Keep the independent semantic and source owners explicit at publication.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_save<A: RevisionAdapter, S: SourceAdapter>(
        &self,
        actor: ObjId,
        name: &str,
        base: EditBase,
        notes: &str,
        version_name: Option<&str>,
        adapter: &A,
        sources: &S,
    ) -> Result<PreparedSave<S::Token>, RollError> {
        let mut mind = self.editable(actor, name, base)?;
        bounded_text("revision notes", notes, MAX_NOTES_BYTES, true)?;
        let version_name = version_name.map(canonical_name).transpose()?;
        if let Some(label) = &version_name {
            if mind.named.contains_key(label) {
                return Err(RollError::Exists(label.clone()));
            }
            if mind.named.len() >= MAX_NAMED_VERSIONS {
                return Err(invalid("versions", "maximum 128 named versions"));
            }
        }
        mind.draft.validate_bounds()?;
        let normalized = adapter.normalize_source(&mind.draft.source)?;
        bounded_source(&normalized)?;
        if adapter.normalize_source(&normalized)? != normalized {
            return Err(invalid("source", "normalizer is not idempotent"));
        }
        adapter.validate(&normalized, &mind.draft.envelope)?;
        let digest = mind.draft.envelope.digest(&normalized)?;
        let source = sources.prepare_source(&normalized, &mind.draft.envelope.mind_abi)?;
        if source.digest != source_digest(&normalized) {
            return Err(invalid("source", "prepared source hash mismatch"));
        }
        if source.operations.len() > 1 {
            return Err(invalid(
                "source",
                "expected at most one immutable source write",
            ));
        }
        for operation in &source.operations {
            encoded(operation, "source operation", MAX_DOCUMENT_BYTES)?;
        }
        let number = mind.next_revision;
        mind.next_revision = number
            .checked_add(1)
            .ok_or(RollError::Exhausted("revision"))?;
        mind.edit = mind
            .edit
            .checked_add(1)
            .ok_or(RollError::Exhausted("edit"))?;
        mind.head = Some(number);
        mind.draft.source = normalized;
        let reference = RevisionRef {
            key: RevisionKey {
                character: actor,
                mind: mind.id,
                number,
            },
            digest,
        };
        if self.occupied.contains(&reference.key.key()) {
            return Err(RollError::Exists(reference.key.key()));
        }
        let revision = Revision {
            format: FORMAT,
            reference,
            source: source.digest,
            envelope: mind.draft.envelope.clone(),
            notes: notes.to_owned(),
        };
        let mut changes = vec![Change::Revision(revision)];
        if let Some(label) = version_name {
            mind.named.insert(label.clone(), number);
            changes.push(Change::Reference(named_reference(
                actor, mind.id, &label, reference,
            )));
        } else {
            mind.unnamed.push_back(number);
            while mind.unnamed.len() > KEEP_UNNAMED {
                mind.unnamed.pop_front();
            }
        }
        changes.push(Change::Mind(mind));
        Ok(PreparedSave {
            change: self.prepare(changes)?,
            source,
            revision: reference,
        })
    }

    pub fn prepare_name_version(
        &self,
        actor: ObjId,
        name: &str,
        base: EditBase,
        number: u64,
        label: &str,
    ) -> Result<PreparedChange, RollError> {
        let mut mind = self.editable(actor, name, base)?;
        let label = canonical_name(label)?;
        if mind.named.contains_key(&label) {
            return Err(RollError::Exists(label));
        }
        if mind.named.len() >= MAX_NAMED_VERSIONS || mind.named.values().any(|n| *n == number) {
            return Err(invalid(
                "versions",
                "named version limit or revision already named",
            ));
        }
        let revision = self
            .revision(
                actor,
                RevisionKey {
                    character: actor,
                    mind: mind.id,
                    number,
                },
            )?
            .reference;
        mind.named.insert(label.clone(), number);
        mind.unnamed.retain(|n| *n != number);
        mind.edit = mind
            .edit
            .checked_add(1)
            .ok_or(RollError::Exhausted("edit"))?;
        let reference = named_reference(actor, mind.id, &label, revision);
        self.prepare(vec![Change::Mind(mind), Change::Reference(reference)])
    }

    pub fn prepare_select(
        &self,
        actor: ObjId,
        selection: Selection,
    ) -> Result<PreparedChange, RollError> {
        let mut library = self
            .libraries
            .get(&actor)
            .cloned()
            .ok_or(RollError::Missing)?;
        let id = match &selection {
            Selection::Pin { revision } => {
                if self.revision(actor, revision.key)?.reference != *revision {
                    return Err(RollError::Missing);
                }
                revision.key.mind
            }
            Selection::Follow { mind } => *mind,
        };
        let mind = self.readable_mind(actor, id)?;
        if mind.head.is_none() {
            return Err(RollError::Missing);
        }
        if !library.names.values().any(|v| *v == mind.id) {
            return Err(RollError::Missing);
        }
        library.selection = Some(selection);
        self.prepare(vec![Change::Library(library)])
    }

    pub fn prepare_rename_version(
        &self,
        actor: ObjId,
        name: &str,
        base: EditBase,
        old: &str,
        new: &str,
    ) -> Result<PreparedChange, RollError> {
        let mut mind = self.editable(actor, name, base)?;
        let old = canonical_name(old)?;
        let new = canonical_name(new)?;
        if mind.named.contains_key(&new) {
            return Err(RollError::Exists(new));
        }
        let number = mind.named.remove(&old).ok_or(RollError::Missing)?;
        mind.named.insert(new.clone(), number);
        mind.edit = mind
            .edit
            .checked_add(1)
            .ok_or(RollError::Exhausted("edit"))?;
        let reference = self
            .revision(
                actor,
                RevisionKey {
                    character: actor,
                    mind: mind.id,
                    number,
                },
            )?
            .reference;
        let old = named_reference(actor, mind.id, &old, reference).key;
        let new = named_reference(actor, mind.id, &new, reference);
        self.prepare(vec![
            Change::Mind(mind),
            Change::RemoveReference(old),
            Change::Reference(new),
        ])
    }

    pub fn prepare_forget_version(
        &self,
        actor: ObjId,
        name: &str,
        base: EditBase,
        label: &str,
    ) -> Result<PreparedChange, RollError> {
        let mut mind = self.editable(actor, name, base)?;
        let label = canonical_name(label)?;
        let number = mind.named.remove(&label).ok_or(RollError::Missing)?;
        let reference = self
            .revision(
                actor,
                RevisionKey {
                    character: actor,
                    mind: mind.id,
                    number,
                },
            )?
            .reference;
        let key = named_reference(actor, mind.id, &label, reference).key;
        mind.unnamed.push_back(number);
        mind.unnamed.make_contiguous().sort_unstable();
        while mind.unnamed.len() > KEEP_UNNAMED {
            mind.unnamed.pop_front();
        }
        mind.edit = mind
            .edit
            .checked_add(1)
            .ok_or(RollError::Exhausted("edit"))?;
        self.prepare(vec![Change::Mind(mind), Change::RemoveReference(key)])
    }

    /// Resolve follow exactly once. The caller embeds this complete owned
    /// revision and every profile into an immutable seal; future edits do not
    /// consult this library again for that running seal.
    pub fn resolve_selection<S: SourceAdapter>(
        &self,
        actor: ObjId,
        sources: &S,
    ) -> Result<Revision, RollError> {
        if self
            .quarantine
            .contains_key(&format!("library:{}", actor.0))
        {
            return Err(RollError::Missing);
        }
        let library = self.libraries.get(&actor).ok_or(RollError::Missing)?;
        let reference = match library.selection.as_ref().ok_or(RollError::Missing)? {
            Selection::Pin { revision } => {
                self.readable_mind(actor, revision.key.mind)?;
                *revision
            }
            Selection::Follow { mind } => {
                let key = self
                    .readable_mind(actor, *mind)?
                    .latest()
                    .ok_or(RollError::Missing)?;
                self.revision(actor, key)?.reference
            }
        };
        let revision = self.revision(actor, reference.key)?;
        if revision.reference != reference {
            return Err(RollError::Missing);
        }
        revision.verify_source(sources)?;
        Ok(revision.clone())
    }

    /// Removal frees the library slot and its working text. Its last fifty
    /// unnamed revisions become one bounded owner reference; named versions and
    /// external references remain roots, even without the library name.
    pub fn prepare_remove(
        &self,
        actor: ObjId,
        name: &str,
        base: EditBase,
    ) -> Result<PreparedChange, RollError> {
        let mind = self.editable(actor, name, base)?;
        let mut library = self
            .libraries
            .get(&actor)
            .cloned()
            .ok_or(RollError::Missing)?;
        library.names.remove(&canonical_name(name)?);
        let selected = match &library.selection {
            Some(Selection::Pin { revision }) => revision.key.mind == mind.id,
            Some(Selection::Follow { mind: id }) => *id == mind.id,
            None => false,
        };
        if selected {
            library.selection = None;
        }
        let mut changes = vec![Change::Library(library), Change::RemoveMind(actor, mind.id)];
        if !mind.unnamed.is_empty() {
            let revisions = mind
                .unnamed
                .iter()
                .map(|number| {
                    self.revision(
                        actor,
                        RevisionKey {
                            character: actor,
                            mind: mind.id,
                            number: *number,
                        },
                    )
                    .map(Revision::reference)
                })
                .collect::<Result<Vec<_>, _>>()?;
            changes.push(Change::Reference(ReferenceRecord {
                format: FORMAT,
                key: ReferenceKey {
                    owner: NS.into(),
                    kind: ReferenceKind::UnnamedHistory,
                    record: format!("{}/{}", actor.0, mind.id),
                },
                revisions,
            }));
        }
        self.prepare(changes)
    }

    /// Enumerate this owner's existing keys without exposing retained revisions
    /// or source. Used to release volatile selection roots on teardown/reboot.
    /// The result is bounded by the store's validated record budget and sorted.
    /// Unreadable records remain quarantined and continue to block pruning.
    pub fn reference_keys(&self, owner: &str, kind: ReferenceKind) -> Vec<ReferenceKey> {
        self.references
            .keys()
            .filter(|key| key.owner == owner && key.kind == kind)
            .cloned()
            .collect()
    }

    pub fn prepare_reference(
        &self,
        owner: &str,
        key: ReferenceKey,
        revisions: Vec<RevisionRef>,
    ) -> Result<PreparedChange, RollError> {
        self.prepare_reference_batch(owner, vec![(key, revisions)], Vec::new())
    }

    pub fn prepare_release_reference(
        &self,
        owner: &str,
        key: &ReferenceKey,
    ) -> Result<PreparedChange, RollError> {
        self.prepare_reference_batch(owner, Vec::new(), vec![key.clone()])
    }

    /// Prepare one atomic retention transaction. Complete validation and
    /// serialization happen before publication, including all exact revisions.
    /// Duplicate keys across either arm refuse; 1..=128 keys may change. A
    /// missing release is Missing, so callers can enumerate existing keys first.
    pub fn prepare_reference_batch(
        &self,
        owner: &str,
        replacements: Vec<(ReferenceKey, Vec<RevisionRef>)>,
        releases: Vec<ReferenceKey>,
    ) -> Result<PreparedChange, RollError> {
        let length = replacements.len().saturating_add(releases.len());
        if length == 0 || length > MAX_REFERENCE_CHANGES {
            return Err(invalid("references", "batch must contain 1..128 keys"));
        }
        let mut seen = BTreeSet::new();
        let mut changes = Vec::with_capacity(length);
        for (key, revisions) in replacements {
            if owner != key.owner
                || owner == NS
                || matches!(
                    key.kind,
                    ReferenceKind::NamedVersion | ReferenceKind::UnnamedHistory
                )
            {
                return Err(invalid(
                    "reference owner",
                    "owner mismatch or reserved retention owner",
                ));
            }
            if !seen.insert(key.clone()) {
                return Err(invalid("references", "duplicate key"));
            }
            let record = ReferenceRecord {
                format: FORMAT,
                key,
                revisions,
            };
            record.validate()?;
            for reference in &record.revisions {
                self.exact(*reference)?;
            }
            changes.push(Change::Reference(record));
        }
        for key in releases {
            if owner != key.owner
                || matches!(
                    key.kind,
                    ReferenceKind::NamedVersion | ReferenceKind::UnnamedHistory
                )
            {
                return Err(invalid(
                    "reference owner",
                    "owner mismatch or reserved retention owner",
                ));
            }
            if !seen.insert(key.clone()) {
                return Err(invalid("references", "duplicate key"));
            }
            if !self.references.contains_key(&key) {
                return Err(RollError::Missing);
            }
            changes.push(Change::RemoveReference(key));
        }
        self.prepare(changes)
    }

    pub fn prepare_prune(&self, limit: usize) -> Result<PreparedPrune, RollError> {
        self.prepare_prune_scope(None, limit)
    }

    /// Explicit author collection is confined to that character's revisions.
    /// Roots and quarantine remain global: a different owner may retain these
    /// revisions, and unreadable external references cannot authorize deletion.
    pub fn prepare_prune_owned(
        &self,
        actor: ObjId,
        limit: usize,
    ) -> Result<PreparedPrune, RollError> {
        self.prepare_prune_scope(Some(actor), limit)
    }

    fn prepare_prune_scope(
        &self,
        actor: Option<ObjId>,
        limit: usize,
    ) -> Result<PreparedPrune, RollError> {
        self.check_collection()?;
        if limit == 0 || limit > MAX_GC_BATCH {
            return Err(invalid("prune", "batch must be 1..128"));
        }
        let roots = self.roots()?;
        let removed: Vec<_> = self
            .revisions
            .values()
            .filter(|r| actor.is_none_or(|actor| r.reference.key.character == actor))
            .filter(|r| !roots.contains(&r.reference.key))
            .take(limit)
            .map(|r| r.reference)
            .collect();
        let removed_keys: BTreeSet<_> = removed.iter().map(|r| r.key).collect();
        let still_used: BTreeSet<_> = self
            .revisions
            .values()
            .filter(|r| !removed_keys.contains(&r.reference.key))
            .map(|r| r.source)
            .collect();
        let source_candidates = removed
            .iter()
            .filter_map(|r| self.revisions.get(&r.key))
            .map(|r| r.source)
            .filter(|hash| !still_used.contains(hash))
            .collect();
        let change = self.prepare(
            removed
                .iter()
                .map(|r| Change::RemoveRevision(r.key))
                .collect(),
        )?;
        Ok(PreparedPrune {
            change,
            source_candidates,
            removed,
        })
    }

    /// Check immediately before entering the final no-failure publication block.
    pub fn check_prepared(&self, prepared: &PreparedChange) -> Result<(), RollError> {
        if self.generation != prepared.generation {
            return Err(RollError::StaleOperation);
        }
        if prepared
            .changes
            .iter()
            .any(|change| matches!(change, Change::RemoveRevision(_)))
        {
            self.check_collection()?;
        }
        self.generation
            .checked_add(1)
            .ok_or(RollError::Exhausted("store generation"))?;
        Ok(())
    }

    /// Recheck first, then publish without any fallible operation. `apply_other`
    /// applies the source owner's already-checked opaque token and queues the
    /// complete combined operation batch; it is called only after this check.
    /// It must not perform I/O, validation, callbacks or return an error.
    pub fn publish_with(
        &mut self,
        prepared: PreparedChange,
        apply_other: impl FnOnce(&[ModuleOp]),
    ) -> Result<(), RollError> {
        self.check_prepared(&prepared)?;
        apply_other(&prepared.operations);
        for change in prepared.changes {
            match change {
                Change::Library(record) => {
                    self.occupied.insert(record.key());
                    self.libraries.insert(record.character, record);
                }
                Change::Mind(record) => {
                    self.occupied.insert(record.key());
                    self.minds.insert((record.character, record.id), record);
                }
                Change::Revision(record) => {
                    self.occupied.insert(record.reference.key.key());
                    self.revisions.insert(record.reference.key, record);
                }
                Change::Reference(record) => {
                    self.occupied.insert(record.key.key());
                    self.references.insert(record.key.clone(), record);
                }
                Change::RemoveMind(character, id) => {
                    self.minds.remove(&(character, id));
                    self.occupied.remove(&format!("mind:{}:{id}", character.0));
                }
                Change::RemoveRevision(key) => {
                    self.revisions.remove(&key);
                    self.occupied.remove(&key.key());
                }
                Change::RemoveReference(key) => {
                    self.references.remove(&key);
                    self.occupied.remove(&key.key());
                }
            }
        }
        self.generation += 1;
        Ok(())
    }

    fn prepare(&self, changes: Vec<Change>) -> Result<PreparedChange, RollError> {
        self.generation
            .checked_add(1)
            .ok_or(RollError::Exhausted("store generation"))?;
        let operations: Vec<_> = changes
            .iter()
            .map(Change::operation)
            .collect::<Result<_, _>>()?;
        let mut count = self.occupied.len();
        let mut changed = BTreeSet::new();
        for operation in &operations {
            let key = match &operation.op {
                ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key } => key,
                _ => return Err(invalid("change", "stream writes forbidden")),
            };
            if self.quarantine.contains_key(key.as_str()) {
                return Err(RollError::Quarantined(key.to_string()));
            }
            if !changed.insert(key.clone()) {
                return Err(invalid("change", "duplicate document write"));
            }
            match &operation.op {
                ModuleWrite::JsonSet { .. } if !self.occupied.contains(key.as_str()) => {
                    count += 1;
                }
                ModuleWrite::Del { .. } if self.occupied.contains(key.as_str()) => {
                    count -= 1;
                }
                _ => {}
            }
        }
        if count > MAX_RECORDS {
            return Err(invalid("roll", "record capacity exhausted"));
        }
        Ok(PreparedChange {
            generation: self.generation,
            changes,
            operations,
        })
    }

    fn editable(&self, actor: ObjId, name: &str, base: EditBase) -> Result<Mind, RollError> {
        let mind = self.mind(actor, name)?;
        if mind.base() != base {
            return Err(RollError::StaleEdit);
        }
        Ok(mind.clone())
    }

    fn exact(&self, reference: RevisionRef) -> Result<&Revision, RollError> {
        let revision = self.revision(reference.key.character, reference.key)?;
        if revision.reference != reference {
            return Err(RollError::Missing);
        }
        Ok(revision)
    }

    fn roots(&self) -> Result<BTreeSet<RevisionKey>, RollError> {
        let mut roots = BTreeSet::new();
        for mind in self.minds.values() {
            for number in mind
                .unnamed
                .iter()
                .chain(mind.named.values())
                .copied()
                .chain(mind.head)
            {
                let key = RevisionKey {
                    character: mind.character,
                    mind: mind.id,
                    number,
                };
                self.revision(mind.character, key)?;
                roots.insert(key);
            }
        }
        for library in self.libraries.values() {
            if let Some(Selection::Pin { revision }) = &library.selection {
                self.exact(*revision)?;
                roots.insert(revision.key);
            }
        }
        for reference in self.references.values() {
            for revision in &reference.revisions {
                self.exact(*revision)?;
                roots.insert(revision.key);
            }
        }
        Ok(roots)
    }

    /// Strict, bounded hydration. Bad keys/documents are quarantined without
    /// deleting or repairing their persisted bytes. No source owner is read
    /// here; composition calls `audit_sources` after every owner has loaded.
    pub fn load(data: &ModuleBootData) -> Result<Self, RollError> {
        if !data.streams.is_empty() || !data.ledger.is_empty() {
            return Err(invalid("roll boot", "unexpected stream or ledger records"));
        }
        if data.kv.len() > MAX_RECORDS {
            return Err(invalid("roll boot", "record capacity exceeded"));
        }
        let mut store = Self::default();
        for (key, value) in &data.kv {
            let key = key.to_string();
            if !store.occupied.insert(key.clone()) {
                store
                    .quarantine
                    .insert(key, "duplicate persistence key".into());
                continue;
            }
            let result = encoded(value, &key, MAX_DOCUMENT_BYTES)
                .and_then(|value| store.load_one(&key, value));
            if let Err(error) = result {
                store.quarantine.insert(key, error.to_string());
            }
        }
        store.check_links();
        Ok(store)
    }

    fn load_one(&mut self, key: &str, value: serde_json::Value) -> Result<(), RollError> {
        fn decode<T: DeserializeOwned>(
            key: &str,
            value: serde_json::Value,
        ) -> Result<T, RollError> {
            serde_json::from_value(value).map_err(|e| invalid(key, e.to_string()))
        }
        if key.starts_with("library:") {
            let record: Library = decode(key, value)?;
            record.validate()?;
            if record.key() != key {
                return Err(invalid(key, "library/key identity mismatch"));
            }
            self.libraries.insert(record.character, record);
        } else if key.starts_with("mind:") {
            let record: Mind = decode(key, value)?;
            record.validate()?;
            if record.key() != key {
                return Err(invalid(key, "mind/key identity mismatch"));
            }
            self.minds.insert((record.character, record.id), record);
        } else if key.starts_with("revision:") {
            let record: Revision = decode(key, value)?;
            record.validate()?;
            if record.reference.key.key() != key {
                return Err(invalid(key, "revision/key identity mismatch"));
            }
            self.revisions.insert(record.reference.key, record);
        } else if key.starts_with("reference:") {
            let record: ReferenceRecord = decode(key, value)?;
            record.validate()?;
            if record.key.key() != key {
                return Err(invalid(key, "reference/key identity mismatch"));
            }
            self.references.insert(record.key.clone(), record);
        } else {
            return Err(invalid(key, "unknown record kind"));
        }
        Ok(())
    }

    fn check_links(&mut self) {
        let mut bad = Vec::new();
        // Persisted key identities count even when their documents cannot be
        // read. A lower allocator must not reissue one of those identities.
        let mut mind_floor: BTreeMap<ObjId, u64> = BTreeMap::new();
        let mut revision_floor: BTreeMap<(ObjId, u64), u64> = BTreeMap::new();
        for key in &self.occupied {
            let mut parts = key.split(':');
            let kind = parts.next();
            if !matches!(kind, Some("mind" | "revision")) {
                continue;
            }
            let Some(character) = parts.next().and_then(|p| p.parse::<u32>().ok()).map(ObjId)
            else {
                continue;
            };
            let Some(mind) = parts.next().and_then(|p| p.parse::<u64>().ok()) else {
                continue;
            };
            mind_floor
                .entry(character)
                .and_modify(|v| *v = (*v).max(mind))
                .or_insert(mind);
            if kind == Some("revision")
                && let Some(number) = parts.next().and_then(|p| p.parse::<u64>().ok())
            {
                revision_floor
                    .entry((character, mind))
                    .and_modify(|v| *v = (*v).max(number))
                    .or_insert(number);
            }
        }
        for library in self.libraries.values() {
            if mind_floor
                .get(&library.character)
                .is_some_and(|floor| library.next_mind <= *floor)
            {
                bad.push((
                    library.key(),
                    "mind allocator would reuse a persisted identity".into(),
                ));
            }
            for (name, id) in &library.names {
                if !self
                    .minds
                    .get(&(library.character, *id))
                    .is_some_and(|m| m.name == *name)
                {
                    bad.push((
                        library.key(),
                        "library names a missing or mismatched mind".into(),
                    ));
                }
            }
            if let Some(Selection::Pin { revision }) = &library.selection
                && self.exact(*revision).is_err()
            {
                bad.push((library.key(), "pin names an unreadable revision".into()));
            }
        }
        for mind in self.minds.values() {
            if revision_floor
                .get(&(mind.character, mind.id))
                .is_some_and(|floor| mind.next_revision <= *floor)
            {
                bad.push((
                    mind.key(),
                    "revision allocator would reuse a persisted identity".into(),
                ));
            }
            if !self
                .libraries
                .get(&mind.character)
                .and_then(|l| l.names.get(&mind.name))
                .is_some_and(|id| *id == mind.id)
            {
                bad.push((mind.key(), "mind has no matching library entry".into()));
            }
            for number in mind
                .unnamed
                .iter()
                .chain(mind.named.values())
                .copied()
                .chain(mind.head)
            {
                if self
                    .revision(
                        mind.character,
                        RevisionKey {
                            character: mind.character,
                            mind: mind.id,
                            number,
                        },
                    )
                    .is_err()
                {
                    bad.push((mind.key(), "retained revision is unreadable".into()));
                }
            }
            for (label, number) in &mind.named {
                let key = RevisionKey {
                    character: mind.character,
                    mind: mind.id,
                    number: *number,
                };
                if let Some(revision) = self.revisions.get(&key) {
                    let expected =
                        named_reference(mind.character, mind.id, label, revision.reference);
                    if self.references.get(&expected.key) != Some(&expected) {
                        bad.push((
                            mind.key(),
                            "named version lacks its retention record".into(),
                        ));
                    }
                }
            }
        }
        for reference in self.references.values() {
            if reference.revisions.iter().any(|r| self.exact(*r).is_err()) {
                bad.push((
                    reference.key.key(),
                    "reference names an unreadable revision".into(),
                ));
            }
        }
        self.quarantine.extend(bad);
    }

    /// Validate loaded executable records through their existing source owner,
    /// holding only shared borrows of both owners. No state or disk is changed.
    pub fn prepare_source_audit<A: RevisionAdapter, S: SourceAdapter>(
        &self,
        adapter: &A,
        sources: &S,
    ) -> Result<PreparedSourceAudit, RollError> {
        self.generation
            .checked_add(1)
            .ok_or(RollError::Exhausted("store generation"))?;
        let mut failures = BTreeMap::new();
        for revision in self.revisions.values() {
            let result = revision.verify_source(sources).and_then(|()| {
                let source = sources.read_source(revision.source)?;
                if adapter.normalize_source(source)? != source {
                    return Err(invalid(
                        "source",
                        "stored revision source is not normalized",
                    ));
                }
                adapter.validate(source, &revision.envelope)
            });
            if let Err(error) = result {
                failures.insert(revision.reference.key.key(), error.to_string());
            }
        }
        Ok(PreparedSourceAudit {
            generation: self.generation,
            failures,
        })
    }

    /// Corruption is quarantined by exact revision key and never rewritten.
    /// The generation check is the last fallible step before publication.
    pub fn apply_source_audit(&mut self, prepared: PreparedSourceAudit) -> Result<(), RollError> {
        if self.generation != prepared.generation {
            return Err(RollError::StaleOperation);
        }
        let next = self
            .generation
            .checked_add(1)
            .ok_or(RollError::Exhausted("store generation"))?;
        self.quarantine.extend(prepared.failures);
        self.check_links();
        // Revalidation is a publication barrier. A prepared prune may not
        // delete anything after this audit finds an unreadable source.
        self.generation = next;
        Ok(())
    }

    /// Convenience for owners not stored in sibling world slots.
    pub fn audit_sources<A: RevisionAdapter, S: SourceAdapter>(
        &mut self,
        adapter: &A,
        sources: &S,
    ) -> Result<(), RollError> {
        let prepared = self.prepare_source_audit(adapter, sources)?;
        self.apply_source_audit(prepared)
    }
}

fn named_reference(
    character: ObjId,
    mind: u64,
    label: &str,
    revision: RevisionRef,
) -> ReferenceRecord {
    // Hex encodes a mutable label so punctuation and spaces never alias a key.
    let label = label
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    ReferenceRecord {
        format: FORMAT,
        key: ReferenceKey {
            owner: NS.into(),
            kind: ReferenceKind::NamedVersion,
            record: format!("{}/{mind}/{label}", character.0),
        },
        revisions: vec![revision],
    }
}

/// Registration only: the game owns commands and supplies its adapters. The
/// caller explicitly lists modules whose save service may queue roll writes.
#[derive(Default)]
pub struct RollModule {
    writers: Vec<String>,
}

impl RollModule {
    pub fn with_writers(writers: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            writers: writers.into_iter().map(Into::into).collect(),
        }
    }
}

impl GameModule for RollModule {
    fn name(&self) -> &'static str {
        NS
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(RollStore::default());
        for writer in &self.writers {
            reg.persist_delegation(
                writer,
                NS,
                "character draft/revision owner API; combined source and revision batch",
            )?;
        }
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let store = RollStore::load(&data).map_err(|e| ModError::msg(e.to_string()))?;
        for (key, reason) in store.quarantine() {
            tracing::error!(%key, %reason, "roll: quarantined record; collection disabled");
        }
        let target = ctx
            .state_mut::<RollStore>()
            .ok_or_else(|| ModError::msg("roll: state slot absent"))?;
        *target = store;
        Ok(())
    }
}
