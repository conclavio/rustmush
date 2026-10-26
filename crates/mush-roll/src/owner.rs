mod authored_retirement;

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mush_core::{
    GameModule, ModError, ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, ObjId, Registrar,
};
use serde::de::DeserializeOwned;
use smol_str::SmolStr;

use crate::commerce::rating_roots::RatingRoots;
use crate::commerce::roots::{CommerceReadiness, InitialRoots, RequiredRoot, RootAnchor};
use crate::commerce::usage::{RollUsage, UsageChange, encoded_len};
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
    sizes: Vec<(String, Option<usize>)>,
    usage: Option<UsageChange>,
    ic_claims: Option<crate::commerce::ic_claims::ClaimInventory>,
    follow: Option<crate::commerce::follow::Prepared>,
}

/// Borrowed only after every composed owner has passed its final check. Dropping
/// this value does nothing; applying it cannot fail, look up state or serialize.
pub(crate) struct BoundCommercialRoots<'a> {
    store: &'a mut RollStore,
    change: PreparedChange,
}

impl BoundCommercialRoots<'_> {
    pub(crate) fn apply(self) {
        self.store.apply_checked(self.change);
    }
}

impl PreparedChange {
    pub(crate) const fn usage(&self) -> Option<UsageChange> {
        self.usage
    }
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
    pub(crate) conduct: crate::commerce::conduct::state::State,
    generation: u64,
    source_audit_epoch: u64,
    commerce: CommerceReadiness,
    // Derived only by the mandatory full owner audit and same-ack closed
    // writer. Default/unloaded absence is never an empty healthy inventory.
    ic_claims: Option<crate::commerce::ic_claims::ClaimInventory>,
    // The value always exists so acknowledged apply needs no fallible lookup.
    // Its separate private latch is set only by the complete owner audit.
    follow: crate::commerce::follow::Index,
    follow_audited: bool,
    libraries: BTreeMap<ObjId, Library>,
    minds: BTreeMap<(ObjId, u64), Mind>,
    revisions: BTreeMap<RevisionKey, Revision>,
    references: BTreeMap<ReferenceKey, ReferenceRecord>,
    /// All occupied keys, including unreadable documents. They cannot be
    /// silently replaced by an operation that assumes absence.
    occupied: BTreeSet<String>,
    /// Exact encoded size for every occupied document; None is unreadable/oversized.
    sizes: BTreeMap<String, Option<usize>>,
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
        if self.commerce == CommerceReadiness::Closed {
            return Err(RollError::Quarantined(
                "commercial retention recovery".into(),
            ));
        }
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
        self.prepare_create_with_provenance(actor, name, draft, None)
    }

    /// Copy an actual owned draft or exact saved revision. The caller cannot
    /// supply or clear provenance. User-facing adapters must separately check
    /// current source rights before invoking this owner operation.
    pub fn prepare_copy<S: SourceAdapter>(
        &self,
        actor: ObjId,
        name: &str,
        from: &str,
        revision: Option<RevisionRef>,
        sources: &S,
    ) -> Result<PreparedChange, RollError> {
        let mind = self.mind(actor, from)?;
        let (draft, provenance) = match revision {
            None => (mind.draft.clone(), mind.provenance.clone()),
            Some(reference) => {
                if reference.key.character != actor || reference.key.mind != mind.id {
                    return Err(RollError::Missing);
                }
                let saved = self.exact(reference)?;
                if saved.provenance != mind.provenance {
                    return Err(RollError::Missing);
                }
                saved.verify_source(sources)?;
                (
                    WorkingCopy {
                        source: sources.read_source(saved.source)?.to_owned(),
                        envelope: saved.envelope.clone(),
                    },
                    saved.provenance.clone(),
                )
            }
        };
        self.prepare_create_with_provenance(actor, name, draft, provenance)
    }

    /// Only the registered fork builder can construct this request. The actual
    /// retained parent and source bytes are rejoined under Roll's owner fence.
    pub(crate) fn prepare_authored_fork(
        &self,
        request: &crate::commerce::authored_fork::AuthoredFork,
    ) -> Result<
        (
            PreparedChange,
            crate::commerce::parent_selector::DraftParentSelection,
        ),
        RollError,
    > {
        self.commerce_ready()?;
        let grant = request.grant();
        let parent = self.exact(grant.holding().revision().reference())?;
        parent.require_original_access()?;
        if parent != grant.holding().revision() {
            return Err(RollError::Missing);
        }
        let provenance = grant.provenance();
        self.check_provenance(&provenance, None)?;
        let draft = request.material().working_copy(parent)?;
        let change = self.prepare_create_with_provenance(
            grant.actor(),
            request.name(),
            draft,
            Some(provenance),
        )?;
        let mind = change
            .changes
            .iter()
            .find_map(|image| match image {
                Change::Mind(mind) => Some(mind),
                _ => None,
            })
            .ok_or(RollError::Missing)?;
        let holder_capture =
            crate::commerce::owner_qualification::records::ParentHolderCapture::capture(grant);
        let selection = crate::commerce::parent_selector::DraftParentSelection {
            format: if holder_capture.is_some() { 2 } else { 1 },
            actor: grant.actor(),
            mind: mind.id,
            parent: grant.provenance().parent,
            charter: grant.holding().charter(),
            line: grant.holding().line(),
            licence: grant.licence(),
            created_parent_term: grant.holding().term(),
            generation: 1,
            created: request.commit(),
            operation: request.commit(),
            holder_capture,
        };
        selection.check_mind(mind).map_err(|_| RollError::Missing)?;
        Ok((change, selection))
    }

    pub(crate) fn derivative_minds(&self) -> impl Iterator<Item = &Mind> {
        self.minds.values().filter(|mind| mind.provenance.is_some())
    }

    fn prepare_create_with_provenance(
        &self,
        actor: ObjId,
        name: &str,
        draft: WorkingCopy,
        provenance: Option<DerivativeProvenance>,
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
            format: if provenance.is_some() {
                DERIVATIVE_FORMAT
            } else {
                FORMAT
            },
            character: actor,
            id,
            name,
            edit: 0,
            next_revision: 1,
            head: None,
            draft,
            provenance,
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
            format: mind.format,
            reference,
            source: source.digest,
            envelope: mind.draft.envelope.clone(),
            notes: notes.to_owned(),
            provenance: mind.provenance.clone(),
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
        // A marked draft is paired with a typed parent selector. Legacy removal
        // cannot retire that row atomically; refuse before either image changes.
        mind.require_original_access()?;
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
                    ReferenceKind::NamedVersion
                        | ReferenceKind::UnnamedHistory
                        | ReferenceKind::Commercial
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
                || owner == NS
                || matches!(
                    key.kind,
                    ReferenceKind::NamedVersion
                        | ReferenceKind::UnnamedHistory
                        | ReferenceKind::Commercial
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
        if self.generation != prepared.generation
            || (prepared.follow.is_some() && !self.follow_audited)
        {
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
        self.apply_checked(prepared);
        Ok(())
    }

    fn apply_checked(&mut self, prepared: PreparedChange) {
        if let Some(follow) = prepared.follow {
            // prepare_follow and check_prepared establish the audited latch
            // before custody; apply neither consults nor creates authority.
            self.follow.apply(follow);
        }
        if let Some(inventory) = prepared.ic_claims {
            self.ic_claims = Some(inventory);
        }
        for (key, size) in prepared.sizes {
            if let Some(size) = size {
                self.sizes.insert(key, Some(size));
            } else {
                self.sizes.remove(&key);
            }
        }
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
    }

    pub(crate) fn conduct_stamp(&self) -> Result<(u64, u64), RollError> {
        self.commerce_ready()?;
        Ok((self.generation, self.source_audit_epoch))
    }

    pub(crate) fn commerce_ready(&self) -> Result<(), RollError> {
        self.check_collection()?;
        match self.commerce {
            CommerceReadiness::Ready { source_epoch }
                if source_epoch != 0 && source_epoch == self.source_audit_epoch =>
            {
                Ok(())
            }
            _ => Err(RollError::Quarantined(
                "commercial retention recovery".into(),
            )),
        }
    }

    pub(crate) fn prepare_commercial_roots(
        &self,
        request: &InitialRoots,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        let mut changes = Vec::with_capacity(request.roots().len());
        for required in request.roots() {
            self.check_root_subject(required)?;
            let record = ReferenceRecord {
                format: FORMAT,
                key: required.anchor.key(),
                revisions: vec![required.target()],
            };
            if self.occupied.contains(&record.key.key()) {
                return Err(invalid("commercial root", "obligation is already retained"));
            }
            changes.push(Change::Reference(record));
        }
        self.prepare(changes)
    }

    pub(crate) fn prepare_line_update_roots(
        &self,
        request: &crate::commerce::line_update::publication::UpdateRoots,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        for required in request.before() {
            self.check_required_root(required)?;
        }
        let mut changes = Vec::with_capacity(request.after().len());
        for required in request.after() {
            self.check_root_subject(required)?;
            let record = ReferenceRecord {
                format: FORMAT,
                key: required.anchor.key(),
                revisions: vec![required.target()],
            };
            if self.occupied.contains(&record.key.key()) {
                return Err(invalid(
                    "compatible update",
                    "obligation is already retained",
                ));
            }
            changes.push(Change::Reference(record));
        }
        self.prepare(changes)
    }

    pub(crate) fn bind_commercial_roots(
        &mut self,
        prepared: PreparedChange,
    ) -> Result<BoundCommercialRoots<'_>, RollError> {
        self.commerce_ready()?;
        self.check_prepared(&prepared)?;
        Ok(BoundCommercialRoots {
            store: self,
            change: prepared,
        })
    }

    /// Exact actual four-owner publication. The permanent root is precreated
    /// on Open and republished unchanged on Close, requiring no fresh capacity.
    pub(crate) fn prepare_ic_claim_roots(
        &self,
        request: &crate::commerce::ic_claims::roots::IcRoots,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        let inventory = self
            .ic_claims
            .as_ref()
            .ok_or_else(|| invalid("IC claim", "final audit unavailable"))?;
        let row = request.row();
        let next = match request.before() {
            None => inventory
                .opening(row.clone())
                .map_err(|_| invalid("IC claim", "admission capacity or identity"))?,
            Some(before) => {
                if inventory.claim(before.open.operation) != Some(before) {
                    return Err(invalid("IC claim", "retained identity differs"));
                }
                inventory
                    .closing(
                        row.closed
                            .clone()
                            .ok_or_else(|| invalid("IC claim", "close missing"))?,
                    )
                    .map_err(|_| invalid("IC claim", "close identity"))?
                    .ok_or_else(|| invalid("IC claim", "event already consumed"))?
            }
        };
        let required = request
            .required()
            .map_err(|_| invalid("IC claim", "root identity"))?;
        self.check_root_subject(&required)?;
        let record = ReferenceRecord {
            format: FORMAT,
            key: required.anchor.key(),
            revisions: vec![required.target()],
        };
        if request.before().is_some() {
            self.check_required_root(&required)?;
        } else if self.occupied.contains(&record.key.key()) {
            return Err(invalid("IC claim", "root already occupied"));
        }
        let mut change = self.prepare(vec![Change::Reference(record)])?;
        change.ic_claims = Some(next);
        Ok(change)
    }

    /// Final actual commerce audit proves an absent IC book is disabled-empty.
    /// Enabled inventory is complete, and both generation/transition are fenced.
    pub(crate) fn charter_transfer_claim_stamp(
        &self,
        charter: crate::commerce::CharterId,
    ) -> Result<Option<(u64, u64)>, RollError> {
        self.commerce_ready()?;
        match &self.ic_claims {
            None => Ok(None),
            Some(book) if !book.open_owned_charter(charter) => {
                Ok(Some((book.generation(), book.next_transition())))
            }
            Some(_) => Err(invalid("charter transfer", "owned dispatch remains open")),
        }
    }
    pub(crate) fn charter_pending_claims(
        &self,
        charter: crate::commerce::CharterId,
    ) -> Result<crate::commerce::charter_pending::records::ClaimObservation, RollError> {
        self.commerce_ready()?;
        let (generation, next_transition, open_owned) = match &self.ic_claims {
            Some(book) => (
                book.generation(),
                book.next_transition(),
                book.owned_charter_now(charter)
                    .map_err(|_| invalid("pending charter", "claim census differs"))?,
            ),
            // Actual final IC audit proves this absent book is disabled-empty.
            None => (0, 1, 0),
        };
        Ok(
            crate::commerce::charter_pending::records::ClaimObservation {
                generation,
                next_transition,
                open_owned,
            },
        )
    }

    /// Actual final audit distinguishes disabled-empty from an unavailable book.
    /// A live claim from any captured holder/term still occupies this licence.
    pub(crate) fn paid_resale_claims(
        &self,
        licence: crate::commerce::LicenceId,
    ) -> Result<crate::commerce::paid_resale::ResaleClaimObservation, RollError> {
        self.commerce_ready()?;
        let (generation, next_transition, open_paid) = match &self.ic_claims {
            Some(book) => (
                book.generation(),
                book.next_transition(),
                u32::try_from(book.paid_open(licence))
                    .map_err(|_| invalid("paid resale", "claim census differs"))?,
            ),
            None => (0, 1, 0),
        };
        Ok(crate::commerce::paid_resale::ResaleClaimObservation {
            generation,
            next_transition,
            open_paid,
        })
    }

    pub(crate) fn prepare_paid_resale_roots(
        &self,
        request: &crate::commerce::paid_resale::publication::ResaleRoots,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        if request.added().is_empty() || request.added().len() > 2 || request.old().len() > 4 {
            return Err(invalid("paid resale", "closed root dimensions"));
        }
        for root in request.old() {
            self.check_required_root(root)?;
        }
        let mut changes = Vec::with_capacity(request.added().len());
        let mut keys = BTreeSet::new();
        for required in request.added() {
            self.check_root_subject(required)?;
            let row = ReferenceRecord {
                format: FORMAT,
                key: required.anchor.key(),
                revisions: vec![required.target()],
            };
            if self.occupied.contains(&row.key.key()) || !keys.insert(row.key.key()) {
                return Err(invalid("paid resale", "cause already retained"));
            }
            changes.push(Change::Reference(row));
        }
        self.prepare(changes)
    }

    pub(crate) fn prepare_charter_pending_roots(
        &self,
        request: &crate::commerce::charter_pending::publication::PendingRoots,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        if request.added().is_empty() || request.added().len() > 3 || request.old().len() > 6 {
            return Err(invalid("pending charter", "closed root dimensions"));
        }
        for root in request.old() {
            self.check_required_root(root)?;
        }
        let mut changes = Vec::with_capacity(request.added().len());
        let mut keys = BTreeSet::new();
        for required in request.added() {
            self.check_root_subject(required)?;
            let row = ReferenceRecord {
                format: FORMAT,
                key: required.anchor.key(),
                revisions: vec![required.target()],
            };
            if self.occupied.contains(&row.key.key()) || !keys.insert(row.key.key()) {
                return Err(invalid("pending charter", "cause already retained"));
            }
            changes.push(Change::Reference(row));
        }
        self.prepare(changes)
    }

    pub(crate) fn prepare_charter_transfer_root(
        &self,
        request: &crate::commerce::charter_transfer::TransferRoots,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        for root in request.old() {
            self.check_required_root(root)?;
        }
        let required = request.added();
        self.check_root_subject(required)?;
        let row = ReferenceRecord {
            format: FORMAT,
            key: required.anchor.key(),
            revisions: vec![required.target()],
        };
        if self.occupied.contains(&row.key.key()) {
            return Err(invalid("charter transfer", "cause already retained"));
        }
        self.prepare(vec![Change::Reference(row)])
    }
    pub(crate) fn ic_claim_next_transition(&self) -> Result<u64, RollError> {
        self.commerce_ready()?;
        self.ic_claims
            .as_ref()
            .map(|book| book.next_transition())
            .ok_or_else(|| invalid("IC claim", "actual composition audit missing"))
    }

    /// Actual final-audited inventory, updated only by the closed four-owner
    /// transition. The caller must still compare its checked typed preimage.
    pub(crate) fn coverage_ic_claim(
        &self,
        operation: mush_core::CommitToken,
    ) -> Result<&crate::commerce::ic_claims::ClaimRecord, RollError> {
        self.commerce_ready()?;
        self.ic_claims
            .as_ref()
            .ok_or_else(|| invalid("IC claim", "actual composition audit missing"))?
            .claim(operation)
            .ok_or_else(|| invalid("IC claim", "acknowledged claim missing"))
    }

    pub(crate) fn follow_index(&self) -> Result<&crate::commerce::follow::Index, RollError> {
        self.commerce_ready()?;
        if !self.follow_audited {
            return Err(invalid("Follow", "complete locator audit missing"));
        }
        Ok(&self.follow)
    }

    pub(crate) fn prepare_follow(
        &self,
        prepared: &mut PreparedChange,
        change: &crate::commerce::follow::Change,
    ) -> Result<(), RollError> {
        self.check_prepared(prepared)?;
        if prepared.follow.is_some() {
            return Err(invalid("Follow", "duplicate locator change"));
        }
        prepared.follow = Some(
            self.follow_index()?
                .prepare(change)
                .map_err(|_| invalid("Follow", "locator and actual transition disagree"))?,
        );
        Ok(())
    }

    pub(crate) fn install_follow_audit(
        &mut self,
        stamp: (u64, u64),
        index: crate::commerce::follow::Index,
    ) -> Result<(), RollError> {
        if self.commerce_audit_base()? != stamp || self.commerce != CommerceReadiness::Closed {
            return Err(invalid("Follow", "stale final audit"));
        }
        self.follow = index;
        self.follow_audited = true;
        Ok(())
    }

    pub(crate) fn install_ic_claim_audit(
        &mut self,
        stamp: (u64, u64),
        inventory: Option<crate::commerce::ic_claims::ClaimInventory>,
    ) -> Result<(), RollError> {
        if self.commerce_audit_base()? != stamp || self.commerce != CommerceReadiness::Closed {
            return Err(invalid("IC claim", "stale final audit"));
        }
        self.ic_claims = inventory;
        Ok(())
    }

    /// Only the closed registered Registry builder can construct this request.
    /// All old roots are checked first, including the permanent job root which
    /// is unchanged. Ordinary prepare enforces the same global record ceiling.
    pub(crate) fn prepare_conduct_root(
        &self,
        request: &crate::commerce::conduct::publication::Roots,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        let required = request.root();
        self.check_root_subject(required)?;
        let record = ReferenceRecord {
            format: FORMAT,
            key: required.anchor.key(),
            revisions: vec![required.target()],
        };
        if request.admission() {
            if self.occupied.contains(&record.key.key()) {
                return Err(invalid("Conduct trial", "obligation already retained"));
            }
        } else {
            self.check_required_root(required)?;
        }
        self.prepare(vec![Change::Reference(record)])
    }

    pub(crate) fn prepare_rating_roots(
        &self,
        request: &RatingRoots,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        for required in request.before() {
            self.check_required_root(required)?;
        }
        for required in request.after() {
            self.check_root_subject(required)?;
        }
        let mut changes = Vec::new();
        for before in request.before() {
            if let Some(after) = request
                .after()
                .iter()
                .find(|after| after.anchor == before.anchor)
            {
                if after.identity != before.identity {
                    return Err(invalid("Registry retention", "changed obligation subject"));
                }
                // Progress still participates in the acknowledged owner union.
                // Re-publish the exact root checked above instead of producing
                // an empty owner image. This adds no key or retained bytes and
                // preserves ordinary generation and capacity validation.
                changes.push(Change::Reference(ReferenceRecord {
                    format: FORMAT,
                    key: before.anchor.key(),
                    revisions: vec![before.target()],
                }));
                continue;
            }
            if !matches!(before.anchor, RootAnchor::RatingPending(_)) {
                return Err(invalid(
                    "Registry retention",
                    "permanent obligation cannot be removed",
                ));
            }
            changes.push(Change::RemoveReference(before.anchor.key()));
        }
        for after in request.after() {
            if request
                .before()
                .iter()
                .any(|before| before.anchor == after.anchor)
            {
                continue;
            }
            let record = ReferenceRecord {
                format: FORMAT,
                key: after.anchor.key(),
                revisions: vec![after.target()],
            };
            if self.occupied.contains(&record.key.key()) {
                return Err(invalid("Registry retention", "obligation already exists"));
            }
            changes.push(Change::Reference(record));
        }
        self.prepare(changes)
    }

    pub(crate) fn check_commercial_prepared(
        &self,
        prepared: &PreparedChange,
    ) -> Result<(), RollError> {
        self.commerce_ready()?;
        self.check_prepared(prepared)
    }

    /// The actual paid allocator reserves both permanent roots at funding.
    /// Terminal grant/refund only verifies them; it cannot require a new Roll
    /// key or rewrite a retained target after escrow has already been funded.
    pub(crate) fn prepare_paid_roots(
        &self,
        request: &crate::commerce::paid_roots::PaidRoots,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        let mut changes = Vec::new();
        if request.before().is_empty() {
            for required in request.after() {
                self.check_root_subject(required)?;
                let record = ReferenceRecord {
                    format: FORMAT,
                    key: required.anchor.key(),
                    revisions: vec![required.target()],
                };
                if self.occupied.contains(&record.key.key()) {
                    return Err(invalid("paid retention", "obligation already exists"));
                }
                changes.push(Change::Reference(record));
            }
        } else {
            if request.before().len() != request.after().len() {
                return Err(invalid("paid retention", "terminal roots differ"));
            }
            for (before, after) in request.before().iter().zip(request.after()) {
                if before.anchor != after.anchor || before.identity != after.identity {
                    return Err(invalid("paid retention", "terminal target differs"));
                }
                self.check_required_root(before)?;
                // The participant remains part of the same acknowledged
                // owner union. Re-publish the identical retained image rather
                // than add a generic zero-write owner bypass. Both keys and
                // their complete byte footprint were admitted at funding.
                changes.push(Change::Reference(ReferenceRecord {
                    format: FORMAT,
                    key: before.anchor.key(),
                    revisions: vec![before.target()],
                }));
            }
        }
        self.prepare(changes)
    }

    /// The closed entitlement request validates listing or funding/terminal
    /// facts. Preserve ordinary root accounting and generation fences.
    pub(crate) fn prepare_house_roots(
        &self,
        request: &crate::commerce::house_purchase::roots::HouseRoots,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        let mut changes = Vec::new();
        if request.before().is_empty() {
            for required in request.after() {
                self.check_root_subject(required)?;
                let record = ReferenceRecord {
                    format: FORMAT,
                    key: required.anchor.key(),
                    revisions: vec![required.target()],
                };
                if self.occupied.contains(&record.key.key()) {
                    return Err(invalid("house retention", "obligation already exists"));
                }
                changes.push(Change::Reference(record));
            }
        } else {
            if request.before().len() != request.after().len() {
                return Err(invalid("house retention", "terminal roots differ"));
            }
            for (before, after) in request.before().iter().zip(request.after()) {
                if before.anchor != after.anchor || before.identity != after.identity {
                    return Err(invalid("house retention", "terminal target differs"));
                }
                self.check_required_root(before)?;
                // The participant remains part of the same acknowledged
                // owner union. Re-publish the identical retained image rather
                // than add a generic zero-write owner bypass. Both keys and
                // their complete byte footprint were admitted at funding.
                changes.push(Change::Reference(ReferenceRecord {
                    format: FORMAT,
                    key: before.anchor.key(),
                    revisions: vec![before.target()],
                }));
            }
        }
        self.prepare(changes)
    }

    pub(crate) fn prepare_entitlement_roots(
        &self,
        request: &crate::commerce::entitlement::roots::EntitlementRoots,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        let mut changes = Vec::new();
        if request.before().is_empty() {
            for required in request.after() {
                self.check_root_subject(required)?;
                let record = ReferenceRecord {
                    format: FORMAT,
                    key: required.anchor.key(),
                    revisions: vec![required.target()],
                };
                if self.occupied.contains(&record.key.key()) {
                    return Err(invalid(
                        "entitlement retention",
                        "obligation already exists",
                    ));
                }
                changes.push(Change::Reference(record));
            }
        } else {
            if request.before().len() != request.after().len() {
                return Err(invalid("entitlement retention", "terminal roots differ"));
            }
            for (before, after) in request.before().iter().zip(request.after()) {
                if before.anchor != after.anchor || before.identity != after.identity {
                    return Err(invalid("entitlement retention", "terminal target differs"));
                }
                self.check_required_root(before)?;
                // The participant remains part of the same acknowledged
                // owner union. Re-publish the identical retained image rather
                // than add a generic zero-write owner bypass. Both keys and
                // their complete byte footprint were admitted at funding.
                changes.push(Change::Reference(ReferenceRecord {
                    format: FORMAT,
                    key: before.anchor.key(),
                    revisions: vec![before.target()],
                }));
            }
        }
        self.prepare(changes)
    }

    /// Only the private renewal request can add its operation and exact next
    /// term roots. A linked retry retains and checks the already admitted term
    /// root; every terminal republishes its identical physical owner image.
    pub(crate) fn prepare_renewal_roots(
        &self,
        request: &crate::commerce::renewal::roots::RenewalRoots,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        let mut seen = std::collections::BTreeSet::new();
        for before in request.before() {
            if !seen.insert(before.anchor.clone())
                || !request
                    .after()
                    .iter()
                    .any(|after| before.anchor == after.anchor && before.identity == after.identity)
            {
                return Err(invalid("renewal retention", "retained obligation changed"));
            }
            self.check_required_root(before)?;
        }
        let mut after_keys = std::collections::BTreeSet::new();
        let mut changes = Vec::new();
        for after in request.after() {
            if !after_keys.insert(after.anchor.clone()) {
                return Err(invalid("renewal retention", "duplicate obligation"));
            }
            self.check_root_subject(after)?;
            let record = ReferenceRecord {
                format: FORMAT,
                key: after.anchor.key(),
                revisions: vec![after.target()],
            };
            if !seen.contains(&after.anchor) && self.occupied.contains(&record.key.key()) {
                return Err(invalid(
                    "renewal retention",
                    "new obligation already exists",
                ));
            }
            changes.push(Change::Reference(record));
        }
        self.prepare(changes)
    }

    /// Only the closed coverage writer can request this immutable source root.
    /// Bind checks the acknowledged IC root; terminal republishes the same
    /// coverage root. Existing IC/filing/rating obligations are never removed.
    pub(crate) fn prepare_coverage_roots(
        &self,
        request: &crate::commerce::coverage::roots::CoverageRoots,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        let mut before_keys = std::collections::BTreeSet::new();
        for before in request.before() {
            if !before_keys.insert(before.anchor.clone()) {
                return Err(invalid("coverage retention", "duplicate preimage"));
            }
            self.check_required_root(before)?;
        }
        let mut after_keys = std::collections::BTreeSet::new();
        let mut changes = Vec::new();
        for after in request.after() {
            if !after_keys.insert(after.anchor.clone()) {
                return Err(invalid("coverage retention", "duplicate obligation"));
            }
            self.check_root_subject(after)?;
            let record = ReferenceRecord {
                format: FORMAT,
                key: after.anchor.key(),
                revisions: vec![after.target()],
            };
            if before_keys.contains(&after.anchor) {
                self.check_required_root(after)?;
            } else if self.occupied.contains(&record.key.key()) {
                return Err(invalid(
                    "coverage retention",
                    "new obligation already exists",
                ));
            }
            changes.push(Change::Reference(record));
        }
        self.prepare(changes)
    }

    pub(crate) fn prepare_house_renewal_roots(
        &self,
        request: &crate::commerce::house_renewal::roots::RenewalRoots,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        let mut seen = std::collections::BTreeSet::new();
        for before in request.before() {
            if !seen.insert(before.anchor.clone())
                || !request
                    .after()
                    .iter()
                    .any(|after| before.anchor == after.anchor && before.identity == after.identity)
            {
                return Err(invalid(
                    "house renewal retention",
                    "retained obligation changed",
                ));
            }
            self.check_required_root(before)?;
        }
        let mut after_keys = std::collections::BTreeSet::new();
        let mut changes = Vec::new();
        for after in request.after() {
            if !after_keys.insert(after.anchor.clone()) {
                return Err(invalid("house renewal retention", "duplicate obligation"));
            }
            self.check_root_subject(after)?;
            let record = ReferenceRecord {
                format: FORMAT,
                key: after.anchor.key(),
                revisions: vec![after.target()],
            };
            if !seen.contains(&after.anchor) && self.occupied.contains(&record.key.key()) {
                return Err(invalid(
                    "house renewal retention",
                    "new obligation already exists",
                ));
            }
            changes.push(Change::Reference(record));
        }
        self.prepare(changes)
    }

    /// New permanent metadata cause, retaining the exact immutable filed subject.
    pub(crate) fn prepare_withdrawal_root(
        &self,
        request: &crate::commerce::withdrawal::Withdrawal,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        self.check_required_root(&request.filed_root())?;
        let required = request
            .cause_root()
            .map_err(|_| invalid("withdrawal retention", "invalid cause"))?;
        self.check_root_subject(&required)?;
        let record = ReferenceRecord {
            format: FORMAT,
            key: required.anchor.key(),
            revisions: vec![required.target()],
        };
        if self.occupied.contains(&record.key.key()) {
            return Err(invalid("withdrawal retention", "cause already retained"));
        }
        self.prepare(vec![Change::Reference(record)])
    }

    /// New permanent metadata cause, retaining the exact immutable filed subject.
    pub(crate) fn prepare_registry_recall_root(
        &self,
        request: &crate::commerce::registry_recall::Recall,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        self.check_required_root(&request.filed_root())?;
        let required = request
            .cause_root()
            .map_err(|_| invalid("Registry recall retention", "invalid cause"))?;
        self.check_root_subject(&required)?;
        let record = ReferenceRecord {
            format: FORMAT,
            key: required.anchor.key(),
            revisions: vec![required.target()],
        };
        if self.occupied.contains(&record.key.key()) {
            return Err(invalid(
                "Registry recall retention",
                "cause already retained",
            ));
        }
        self.prepare(vec![Change::Reference(record)])
    }

    pub(crate) fn prepare_measured_recall_root(
        &self,
        request: &crate::commerce::rating::reconformance::recall::publication::Recall,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        self.check_required_root(&request.filed_root())?;
        let required = request
            .cause_root()
            .map_err(|_| invalid("Registry recall retention", "invalid cause"))?;
        self.check_root_subject(&required)?;
        let record = ReferenceRecord {
            format: FORMAT,
            key: required.anchor.key(),
            revisions: vec![required.target()],
        };
        if self.occupied.contains(&record.key.key()) {
            return Err(invalid(
                "Registry recall retention",
                "cause already retained",
            ));
        }
        self.prepare(vec![Change::Reference(record)])
    }

    /// New permanent metadata cause, retaining the exact immutable filed subject.
    pub(crate) fn prepare_registry_revocation_root(
        &self,
        request: &crate::commerce::registry_revocation::Revocation,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        self.check_required_root(&request.filed_root())?;
        let mut added = vec![
            request
                .cause_root()
                .map_err(|_| invalid("Registry revocation retention", "invalid cause"))?,
        ];
        if let Some(pending) = request.pending() {
            if pending.commit() != request.commit()
                || pending.added().len() != 1
                || pending.old().len() != 2
            {
                return Err(invalid(
                    "Registry revocation retention",
                    "invalid compound refund",
                ));
            }
            for root in pending.old() {
                self.check_required_root(root)?;
            }
            added.extend_from_slice(pending.added());
        }
        if let Some(resale) = request.resale() {
            if request.pending().is_some()
                || resale.commit() != request.commit()
                || resale.old().len() != 1
                || resale.added().len() != 1
            {
                return Err(invalid(
                    "Registry revocation retention",
                    "invalid paid resale refund",
                ));
            }
            for root in resale.old() {
                self.check_required_root(root)?;
            }
            added.extend_from_slice(resale.added());
        }
        let mut changes = Vec::with_capacity(added.len());
        let mut keys = BTreeSet::new();
        for required in added {
            self.check_root_subject(&required)?;
            let record = ReferenceRecord {
                format: FORMAT,
                key: required.anchor.key(),
                revisions: vec![required.target()],
            };
            if self.occupied.contains(&record.key.key()) || !keys.insert(record.key.key()) {
                return Err(invalid(
                    "Registry revocation retention",
                    "cause already retained",
                ));
            }
            changes.push(Change::Reference(record));
        }
        self.prepare(changes)
    }

    /// A new generation retains its exact child and selected-parent evidence
    /// without changing any owned source, old term or existing custody root.
    pub(crate) fn prepare_owner_attestation_roots(
        &self,
        request: &crate::commerce::owner_attestation::Attestation,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        for root in request.existing_roots() {
            self.check_required_root(&root)?;
        }
        let mut changes = Vec::new();
        for required in request
            .roots()
            .map_err(|_| invalid("owner attestation", "invalid cause"))?
        {
            self.check_root_subject(&required)?;
            let record = ReferenceRecord {
                format: FORMAT,
                key: required.anchor.key(),
                revisions: vec![required.target()],
            };
            if self.occupied.contains(&record.key.key()) {
                return Err(invalid("owner attestation", "cause exists"));
            }
            changes.push(Change::Reference(record));
        }
        self.prepare(changes)
    }

    /// A current qualification changes no owned source or old root. Its exact
    /// permanent cause retains the selected revision under normal owner limits.
    pub(crate) fn prepare_owner_qualification_root(
        &self,
        request: &crate::commerce::owner_qualification::Qualification,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        for root in request.original_roots() {
            self.check_required_root(&root)?;
        }
        let required = request
            .cause_root()
            .map_err(|_| invalid("owner qualification", "invalid cause"))?;
        self.check_root_subject(&required)?;
        let record = ReferenceRecord {
            format: FORMAT,
            key: required.anchor.key(),
            revisions: vec![required.target()],
        };
        if self.occupied.contains(&record.key.key()) {
            return Err(invalid("owner qualification", "cause exists"));
        }
        self.prepare(vec![Change::Reference(record)])
    }

    /// Public historical fact only; the complete typed/E4/root audit has already
    /// joined this exact Filed anchor. No source, current eligibility or scan.
    pub(crate) fn require_public_filed_fact(
        &self,
        filed: crate::commerce::FiledRevisionId,
    ) -> Result<(), RollError> {
        self.commerce_ready()?;
        let key = RootAnchor::Filed(filed).key();
        let row = self.references.get(&key).ok_or(RollError::Missing)?;
        let [reference] = row.revisions.as_slice() else {
            return Err(invalid("filed fact", "invalid root"));
        };
        self.exact(*reference)?;
        Ok(())
    }
    pub(crate) fn prepare_term_lapse_root(
        &self,
        request: &crate::commerce::facts::publication::Lapse,
    ) -> Result<PreparedChange, RollError> {
        self.commerce_ready()?;
        let required = request
            .root()
            .map_err(|_| invalid("term lapse", "invalid occurrence"))?;
        self.check_root_subject(&required)?;
        let record = ReferenceRecord {
            format: FORMAT,
            key: required.anchor.key(),
            revisions: vec![required.target()],
        };
        if self.occupied.contains(&record.key.key()) {
            return Err(invalid("term lapse", "occurrence exists"));
        }
        self.prepare(vec![Change::Reference(record)])
    }

    /// Read-only final audit prerequisite. It deliberately accepts either final
    /// hook order, without opening the commercial latch or issuing live rights.
    pub(crate) fn coverage_audit_ready(&self) -> Result<(), RollError> {
        if self.commerce == CommerceReadiness::Disabled
            || self.source_audit_epoch == 0
            || !self.quarantine.is_empty()
        {
            return Err(RollError::Quarantined("coverage retention recovery".into()));
        }
        Ok(())
    }

    pub(crate) fn check_root_subject(&self, required: &RequiredRoot) -> Result<(), RollError> {
        let revision = self.exact(required.target())?;
        if !required
            .identity
            .matches(revision)
            .map_err(|_| invalid("commercial root", "invalid subject"))?
        {
            return Err(invalid("commercial root", "source identity mismatch"));
        }
        Ok(())
    }

    pub(crate) fn check_required_root(&self, required: &RequiredRoot) -> Result<(), RollError> {
        self.check_root_subject(required)?;
        let record = self
            .references
            .get(&required.anchor.key())
            .ok_or(RollError::Missing)?;
        if record.revisions.as_slice() != [required.target()] {
            return Err(invalid("commercial root", "obligation target mismatch"));
        }
        Ok(())
    }

    pub(crate) fn commercial_roots(
        &self,
    ) -> impl Iterator<Item = Result<(RootAnchor, RevisionRef), RollError>> + '_ {
        self.references
            .values()
            .filter(|row| row.key.kind == ReferenceKind::Commercial)
            .map(|row| {
                let anchor = RootAnchor::parse(&row.key.record)
                    .map_err(|_| invalid("commercial root", "invalid anchor"))?;
                let [revision] = row.revisions.as_slice() else {
                    return Err(invalid("commercial root", "invalid count"));
                };
                Ok((anchor, *revision))
            })
    }

    /// Invoked before reading any commercial family, including a zero-root
    /// installation. Invalidates already prepared collection even on failure.
    pub(crate) fn close_commerce_for_audit(&mut self) -> Result<(), RollError> {
        if self.commerce == CommerceReadiness::Disabled {
            return Err(invalid("commerce", "owner composition is absent"));
        }
        let next = self
            .generation
            .checked_add(1)
            .ok_or(RollError::Exhausted("store generation"))?;
        self.commerce = CommerceReadiness::Closed;
        self.follow = crate::commerce::follow::Index::default();
        self.follow_audited = false;
        self.generation = next;
        Ok(())
    }

    pub(crate) fn commerce_audit_base(&self) -> Result<(u64, u64), RollError> {
        if self.commerce != CommerceReadiness::Closed
            || self.source_audit_epoch == 0
            || !self.quarantine.is_empty()
        {
            return Err(RollError::Quarantined(
                "commercial retention recovery".into(),
            ));
        }
        Ok((self.generation, self.source_audit_epoch))
    }

    /// The closed first-install provider must also prove the typed families,
    /// shell deployment and journal-derived filing history empty. This check
    /// covers only this owner's actual retained roots; it does not initialize
    /// the deployment or open collection/business readiness.
    pub(crate) fn commerce_initialization_base(&self) -> Result<(u64, u64), RollError> {
        let stamp = self.commerce_audit_base()?;
        if self
            .references
            .values()
            .any(|row| row.key.kind == ReferenceKind::Commercial)
        {
            return Err(invalid("commerce", "commercial retention already exists"));
        }
        Ok(stamp)
    }

    /// Only the synchronous complete typed-family audit can invoke this. No
    /// public guard-clear API and no partial-family success path exist.
    pub(crate) fn finish_commerce_audit(
        &mut self,
        generation: u64,
        source_epoch: u64,
    ) -> Result<(), RollError> {
        if self.commerce_audit_base()? != (generation, source_epoch) {
            return Err(RollError::StaleOperation);
        }
        let next = self
            .generation
            .checked_add(1)
            .ok_or(RollError::Exhausted("store generation"))?;
        self.commerce = CommerceReadiness::Ready { source_epoch };
        self.generation = next;
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
        let mut sizes = Vec::with_capacity(operations.len());
        let mut changed = BTreeSet::new();
        for operation in &operations {
            let key = match &operation.op {
                ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key } => key,
                _ => return Err(invalid("change", "stream writes forbidden")),
            };
            let size = match &operation.op {
                ModuleWrite::JsonSet { value, .. } => Some(
                    encoded_len(value)
                        .ok_or_else(|| invalid("change", "document size unavailable"))?,
                ),
                ModuleWrite::Del { .. } => None,
                _ => return Err(invalid("change", "unsupported size accounting")),
            };
            sizes.push((key.to_string(), size));
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
        let usage = self.current_usage().and_then(|before| {
            let mut after = before;
            for (key, next) in &sizes {
                if let Some(old) = self.sizes.get(key) {
                    after.document_bytes = after.document_bytes.checked_sub((*old)? as u64)?;
                    after.key_bytes = after.key_bytes.checked_sub(key.len() as u64)?;
                }
                if let Some(next) = next {
                    after.document_bytes = after.document_bytes.checked_add(*next as u64)?;
                    after.key_bytes = after.key_bytes.checked_add(key.len() as u64)?;
                }
            }
            after.records = count;
            Some(UsageChange { before, after })
        });
        Ok(PreparedChange {
            generation: self.generation,
            changes,
            operations,
            sizes,
            usage,
            ic_claims: None,
            follow: None,
        })
    }

    pub(crate) fn current_usage(&self) -> Option<RollUsage> {
        if self.sizes.len() != self.occupied.len() {
            return None;
        }
        let mut usage = RollUsage {
            records: self.occupied.len(),
            document_bytes: 0,
            key_bytes: 0,
        };
        for (key, bytes) in &self.sizes {
            usage.document_bytes = usage.document_bytes.checked_add((*bytes)? as u64)?;
            usage.key_bytes = usage.key_bytes.checked_add(key.len() as u64)?;
        }
        Some(usage)
    }

    fn editable(&self, actor: ObjId, name: &str, base: EditBase) -> Result<Mind, RollError> {
        let mind = self.mind(actor, name)?;
        if mind.base() != base {
            return Err(RollError::StaleEdit);
        }
        Ok(mind.clone())
    }

    pub(crate) fn exact(&self, reference: RevisionRef) -> Result<&Revision, RollError> {
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
        // Conservatively retain exact source parents of every retained marked
        // record. A later prune may release a parent only after its last child
        // record disappears; no second root registry or client root is created.
        for provenance in self
            .minds
            .values()
            .filter_map(|m| m.provenance.as_ref())
            .chain(
                self.revisions
                    .values()
                    .filter_map(|r| r.provenance.as_ref()),
            )
        {
            let parent = self.exact(provenance.parent.revision.revision())?;
            if !provenance
                .parent
                .revision
                .matches(parent)
                .map_err(|_| RollError::Missing)?
            {
                return Err(RollError::Missing);
            }
            roots.insert(parent.reference.key);
        }
        Ok(roots)
    }

    /// Strict, bounded hydration. Bad keys/documents are quarantined without
    /// deleting or repairing their persisted bytes. No source owner is read
    /// here; composition calls `audit_sources` after every owner has loaded.
    pub fn load(data: &ModuleBootData) -> Result<Self, RollError> {
        Self::load_mode(data, false)
    }

    /// Only the real composed Roll module uses this path. A retained commercial
    /// installation starts closed, including when every root disappeared.
    pub(crate) fn load_commerce(data: &ModuleBootData) -> Result<Self, RollError> {
        Self::load_mode(data, true)
    }

    fn load_mode(data: &ModuleBootData, commerce: bool) -> Result<Self, RollError> {
        if !data.streams.is_empty() || !data.ledger.is_empty() {
            return Err(invalid("roll boot", "unexpected stream or ledger records"));
        }
        if data.kv.len() > MAX_RECORDS {
            return Err(invalid("roll boot", "record capacity exceeded"));
        }
        let mut store = Self {
            commerce: if commerce {
                CommerceReadiness::Closed
            } else {
                CommerceReadiness::Disabled
            },
            ..Self::default()
        };
        for (key, value) in &data.kv {
            let key = key.to_string();
            if !store.occupied.insert(key.clone()) {
                store
                    .quarantine
                    .insert(key, "duplicate persistence key".into());
                continue;
            }
            store.sizes.insert(key.clone(), encoded_len(value));
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
            if record.key.kind == ReferenceKind::Commercial
                && self.commerce == CommerceReadiness::Disabled
            {
                return Err(invalid(
                    "commercial root",
                    "commercial owner composition is absent",
                ));
            }
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
        for revision in self.revisions.values() {
            if self
                .minds
                .get(&(
                    revision.reference.key.character,
                    revision.reference.key.mind,
                ))
                .is_some_and(|mind| mind.provenance != revision.provenance)
            {
                bad.push((
                    revision.reference.key.key(),
                    "revision ancestry differs from its mind".into(),
                ));
                if let Some(mind) = self.minds.get(&(
                    revision.reference.key.character,
                    revision.reference.key.mind,
                )) {
                    bad.push((mind.key(), "mind ancestry differs from its revision".into()));
                }
            }
        }
        for (key, provenance, own) in self
            .minds
            .values()
            .filter_map(|m| m.provenance.as_ref().map(|p| (m.key(), p, None)))
            .chain(self.revisions.values().filter_map(|r| {
                r.provenance
                    .as_ref()
                    .map(|p| (r.reference.key.key(), p, Some(r.reference)))
            }))
        {
            if self.check_provenance(provenance, own).is_err() {
                bad.push((
                    key,
                    "ancestry is missing, mismatched, cyclic or too deep".into(),
                ));
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

    /// Structural source retention only. The future closed fork/filing owner
    /// must additionally join these identities to current commercial rights.
    fn check_provenance(
        &self,
        provenance: &DerivativeProvenance,
        own: Option<RevisionRef>,
    ) -> Result<(), RollError> {
        let mut seen = BTreeSet::new();
        if let Some(own) = own {
            seen.insert(own.key);
        }
        let mut next = Some(provenance);
        for _ in 0..crate::commerce::filed::MAX_PARENT_DEPTH {
            let Some(value) = next else {
                return Ok(());
            };
            let reference = value.parent.revision.revision();
            if !seen.insert(reference.key) {
                return Err(RollError::Missing);
            }
            let parent = self.exact(reference)?;
            if !value
                .parent
                .revision
                .matches(parent)
                .map_err(|_| RollError::Missing)?
            {
                return Err(RollError::Missing);
            }
            next = parent.provenance.as_ref();
        }
        if next.is_some() {
            Err(RollError::Missing)
        } else {
            Ok(())
        }
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
        let next_source_epoch = self
            .source_audit_epoch
            .checked_add(1)
            .ok_or(RollError::Exhausted("source audit epoch"))?;
        self.quarantine.extend(prepared.failures);
        self.check_links();
        // Revalidation is a publication barrier. A prepared prune may not
        // delete anything after this audit finds an unreadable source.
        self.generation = next;
        self.source_audit_epoch = next_source_epoch;
        if self.commerce != CommerceReadiness::Disabled {
            self.commerce = CommerceReadiness::Closed;
        }
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
    commerce: Option<Box<crate::commerce::module::CommerceConfig>>,
    registration: Option<Box<crate::commerce::module::CommerceRegistration>>,
    registry_owners: Option<Box<dyn crate::commerce::rating::registration::Installer>>,
    fork_source: Option<Box<dyn crate::commerce::authored_fork::Installer>>,
}

impl RollModule {
    pub fn with_writers(writers: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            writers: writers.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }

    /// Opt in to the closed, registered commercial composition. Existing save
    /// delegations remain intact; this does not itself open business readiness.
    pub fn with_commerce(mut self, config: crate::commerce::module::CommerceConfig) -> Self {
        self.commerce = Some(Box::new(config));
        self
    }

    /// Trusted composition chooses the two actual game-owned participant
    /// types. This cannot accept a callback, handle, measurement or authority
    /// from a command. Registration refuses Initialize or absent commerce.
    pub fn with_registry_owners<Q, A>(mut self) -> Self
    where
        Q: crate::commerce::rating::protocol::RegistryQuotaParticipant,
        A: mush_core::participant::OwnerParticipant<
                Request = crate::commerce::rating::protocol::ArtifactRequest,
                Output = crate::commerce::rating::protocol::ArtifactOutput,
            >,
    {
        self.registry_owners = Some(crate::commerce::rating::registration::owners::<Q, A>());
        self
    }

    /// Trusted opt-in selects the actual immutable source read service type.
    /// No callbacks, source strings, client authority or ready flags are accepted.
    pub fn with_authored_fork_source<S>(mut self) -> Self
    where
        S: mush_core::read_service::OwnerReadService<
                Request = crate::commerce::fork_protocol::ForkMaterialRequest,
                Output = crate::commerce::fork_protocol::ForkMaterial,
            >,
    {
        self.fork_source = Some(crate::commerce::authored_fork::source::<S>());
        self
    }

    /// Captures actual registered handles for the composed Engine fixture.
    /// This neither constructs a registry nor changes commercial readiness.
    #[cfg(test)]
    pub(crate) fn commerce_test_handles(
        &self,
    ) -> Option<(
        crate::commerce::families::CommerceFamilies,
        crate::commerce::rating::RatingFamilies,
    )> {
        self.registration
            .as_ref()
            .map(|r| (r.families.clone(), r.ratings.clone()))
    }
}

impl GameModule for RollModule {
    fn name(&self) -> &'static str {
        NS
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(if self.commerce.is_some() {
            RollStore::load_commerce(&ModuleBootData::default())
                .map_err(|e| ModError::msg(e.to_string()))?
        } else {
            RollStore::default()
        });
        for writer in &self.writers {
            reg.persist_delegation(
                writer,
                NS,
                "character draft/revision owner API; combined source and revision batch",
            )?;
        }
        if let Some(config) = &self.commerce {
            self.registration = Some(Box::new(crate::commerce::module::register(
                reg,
                config,
                self.registry_owners.as_deref(),
                self.fork_source.as_deref(),
            )?));
        } else if self.registry_owners.is_some() || self.fork_source.is_some() {
            return Err(ModError::msg(
                "Registry owners require explicit Active commerce",
            ));
        }
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let store = if self.commerce.is_some() {
            RollStore::load_commerce(&data)
        } else {
            RollStore::load(&data)
        }
        .map_err(|e| ModError::msg(e.to_string()))?;
        for (key, reason) in store.quarantine() {
            tracing::error!(%key, %reason, "roll: quarantined record; collection disabled");
        }
        let target = ctx
            .state_mut::<RollStore>()
            .ok_or_else(|| ModError::msg("roll: state slot absent"))?;
        *target = store;
        if let Some(registration) = &self.registration {
            registration.configure_conduct(target)?;
        }
        Ok(())
    }

    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        match (&self.commerce, &self.registration) {
            (Some(_), Some(registration)) => {
                registration.finalize(ctx, self.registry_owners.as_deref())
            }
            (None, None) => Ok(()),
            _ => Err(ModError::msg("roll: commercial registration is incomplete")),
        }
    }
}
