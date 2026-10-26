//! Metadata-only owner mutation. No source adapter or current paid-rights resolver.
use super::*;
use crate::commerce::authored_retirement::records::{
    NamedRetention, RetirementOperation, selection_digest,
};
use crate::commerce::authored_retirement::{Retirement, RetirementOutput};
use crate::commerce::records::Record;

impl RollStore {
    pub(crate) fn prepare_authored_retirement(
        &self,
        request: &Retirement,
    ) -> Result<(PreparedChange, RetirementOutput), RollError> {
        self.commerce_ready()?;
        let subject = &request.subject;
        let mind = self.editable(subject.actor, &subject.name, subject.base)?;
        if mind.id != subject.mind {
            return Err(RollError::Missing);
        }
        subject
            .selector
            .check_mind(&mind)
            .map_err(|_| RollError::Missing)?;
        let reference = |number| {
            self.revision(
                subject.actor,
                RevisionKey {
                    character: subject.actor,
                    mind: subject.mind,
                    number,
                },
            )
            .map(Revision::reference)
        };
        let unnamed = mind
            .unnamed
            .iter()
            .map(|n| reference(*n))
            .collect::<Result<Vec<_>, _>>()?;
        let named = mind
            .named
            .iter()
            .map(|(name, number)| {
                Ok(NamedRetention {
                    name: name.clone(),
                    revision: reference(*number)?,
                })
            })
            .collect::<Result<Vec<_>, RollError>>()?;
        let operation = RetirementOperation {
            format: 1,
            actor: subject.actor,
            mind: subject.mind,
            selection: selection_digest(&subject.selector).map_err(|_| RollError::Missing)?,
            operation: request.commit,
            base: subject.base,
            action: request.action.clone(),
            name: mind.name.clone(),
            parent: subject.selector.parent.revision,
            unnamed,
            named,
        };
        operation.validate().map_err(|_| RollError::Missing)?;
        // Check all immutable named retention against the actual owner map.
        self.check_retirement_named(&operation)?;
        let required = RequiredRoot {
            anchor: RootAnchor::DraftRetirement(
                request.commit,
                operation.digest().map_err(|_| RollError::Missing)?,
            ),
            identity: operation.parent,
        };
        self.check_root_subject(&required)?;
        if self.occupied.contains(&required.anchor.key().key())
            || self
                .references
                .contains_key(&retirement_unnamed_key(operation.actor, operation.mind))
        {
            return Err(RollError::Missing);
        }
        let mut library = self
            .libraries
            .get(&subject.actor)
            .cloned()
            .ok_or(RollError::Missing)?;
        if library.names.remove(&mind.name) != Some(mind.id) {
            return Err(RollError::Missing);
        }
        if library.selection.as_ref().is_some_and(|s| match s {
            Selection::Pin { revision } => revision.key.mind == mind.id,
            Selection::Follow { mind: id } => *id == mind.id,
        }) {
            library.selection = None;
        }
        let mut changes = vec![
            Change::Library(library),
            Change::RemoveMind(subject.actor, mind.id),
            Change::Reference(ReferenceRecord {
                format: FORMAT,
                key: required.anchor.key(),
                revisions: vec![required.target()],
            }),
        ];
        if !operation.unnamed.is_empty() {
            changes.push(Change::Reference(ReferenceRecord {
                format: FORMAT,
                key: retirement_unnamed_key(operation.actor, operation.mind),
                revisions: operation.unnamed.clone(),
            }));
        }
        let tombstone = operation.tombstone().map_err(|_| RollError::Missing)?;
        // Ordinary preparation checks the complete owner record/key/byte limits.
        Ok((
            self.prepare(changes)?,
            RetirementOutput {
                operation,
                tombstone,
            },
        ))
    }

    pub(crate) fn check_authored_retirement(
        &self,
        operation: &RetirementOperation,
        selector: &crate::commerce::parent_selector::DraftParentSelection,
    ) -> Result<(), RollError> {
        if !self.quarantine.is_empty() || self.source_audit_epoch == 0 {
            return Err(RollError::Missing);
        }
        operation.validate().map_err(|_| RollError::Missing)?;
        let library = self
            .libraries
            .get(&operation.actor)
            .ok_or(RollError::Missing)?;
        if self.minds.contains_key(&(operation.actor, operation.mind))
            || self
                .occupied
                .contains(&format!("mind:{}:{}", operation.actor.0, operation.mind))
            || library.names.values().any(|id| *id == operation.mind)
            || library.next_mind <= operation.mind
            || library.selection.as_ref().is_some_and(|s| match s {
                Selection::Pin { revision } => revision.key.mind == operation.mind,
                Selection::Follow { mind } => *mind == operation.mind,
            })
        {
            return Err(RollError::Missing);
        }
        self.check_retirement_named(operation)?;
        let actual = self
            .references
            .get(&retirement_unnamed_key(operation.actor, operation.mind));
        match (operation.unnamed.is_empty(), actual) {
            (true, None) => {}
            (false, Some(row)) if row.revisions == operation.unnamed => {}
            _ => return Err(RollError::Missing),
        }
        for revision in operation
            .unnamed
            .iter()
            .chain(operation.named.iter().map(|entry| &entry.revision))
        {
            if self.exact(*revision)?.provenance.as_ref() != Some(&selector.provenance()) {
                return Err(RollError::Missing);
            }
        }
        self.check_required_root(&RequiredRoot {
            anchor: RootAnchor::DraftRetirement(
                operation.operation,
                operation.digest().map_err(|_| RollError::Missing)?,
            ),
            identity: operation.parent,
        })
    }
    fn check_retirement_named(&self, operation: &RetirementOperation) -> Result<(), RollError> {
        for entry in &operation.named {
            self.exact(entry.revision)?;
            let expected =
                named_reference(operation.actor, operation.mind, &entry.name, entry.revision);
            if self.references.get(&expected.key) != Some(&expected) {
                return Err(RollError::Missing);
            }
        }
        // Reject additional named retention for this exact retired identity.
        let prefix = format!("{}/{}/", operation.actor.0, operation.mind);
        let count = self
            .references
            .values()
            .filter(|row| {
                row.key.owner == NS
                    && row.key.kind == ReferenceKind::NamedVersion
                    && row.key.record.starts_with(&prefix)
            })
            .count();
        if count != operation.named.len() {
            return Err(RollError::Missing);
        }
        Ok(())
    }
}
fn retirement_unnamed_key(actor: ObjId, mind: u64) -> ReferenceKey {
    ReferenceKey {
        owner: NS.into(),
        kind: ReferenceKind::UnnamedHistory,
        record: format!("{}/{mind}", actor.0),
    }
}
