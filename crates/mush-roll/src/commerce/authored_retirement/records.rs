//! Immutable metadata-only retirement evidence. No paid/source permission.
use super::super::RevisionIdentity;
use super::super::parent_selector::DraftParentSelection;
use super::super::records::{PublicationError, Record};
use crate::{EditBase, RevisionRef};
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
fn invalid<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}
fn bounded<T: Serialize>(value: &T, limit: usize) -> Result<serde_json::Value, PublicationError> {
    let value = serde_json::to_value(value).map_err(invalid)?;
    mush_core::canonical_json_bytes_with_limit(&value, limit)
        .map_err(|_| PublicationError::Capacity)?;
    Ok(value)
}
pub(crate) fn selection_digest(
    selection: &DraftParentSelection,
) -> Result<DefinitionDigest, PublicationError> {
    selection.validate()?;
    DefinitionDigest::for_canonical_value(
        "roll.authored_retirement.selection.v1",
        &bounded(selection, DraftParentSelection::MAX_BYTES)?,
    )
    .map_err(invalid)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NamedRetention {
    pub(crate) name: String,
    pub(crate) revision: RevisionRef,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetirementOperation {
    pub(crate) format: u16,
    pub(crate) actor: ObjId,
    pub(crate) mind: u64,
    pub(crate) selection: DefinitionDigest,
    pub(crate) operation: CommitToken,
    pub(crate) base: EditBase,
    pub(crate) action: DefinitionRef,
    pub(crate) name: String,
    pub(crate) parent: RevisionIdentity,
    #[serde(deserialize_with = "unnamed_inventory")]
    pub(crate) unnamed: Vec<RevisionRef>,
    #[serde(deserialize_with = "named_inventory")]
    pub(crate) named: Vec<NamedRetention>,
}
impl RetirementOperation {
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "roll.authored_retirement.operation.v1",
            &bounded(self, Self::MAX_BYTES)?,
        )
        .map_err(invalid)
    }
    pub(crate) fn tombstone(&self) -> Result<RetiredDraftParent, PublicationError> {
        self.validate()?;
        Ok(RetiredDraftParent {
            format: 1,
            actor: self.actor,
            mind: self.mind,
            selection: self.selection,
            operation: self.operation,
            base: self.base,
            action: self.action.clone(),
        })
    }
    pub(crate) fn check_selection(
        &self,
        selection: &DraftParentSelection,
    ) -> Result<(), PublicationError> {
        if self.actor != selection.actor
            || self.mind != selection.mind
            || self.selection != selection_digest(selection)?
            || self.parent != selection.parent.revision
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
impl Record for RetirementOperation {
    const FAMILY: &'static str = "roll_mind_authored_retirement";
    const MAX_BYTES: usize = 65536;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1
            || self.mind == 0
            || self.base.revision == Some(0)
            || self.base.revision.is_some_and(|n| n > self.base.edit)
            || crate::canonical_name(&self.name).map_err(invalid)? != self.name
            || !super::known_action(&self.action).map_err(invalid)?
            || self.unnamed.len() > crate::KEEP_UNNAMED
            || self.named.len() > crate::MAX_NAMED_VERSIONS
        {
            return Err(PublicationError::Invalid);
        }
        let mut numbers = BTreeSet::new();
        let mut previous = 0;
        for revision in &self.unnamed {
            if revision.key.character != self.actor
                || revision.key.mind != self.mind
                || revision.key.number <= previous
                || self.base.revision.is_none_or(|n| revision.key.number > n)
                || !numbers.insert(revision.key.number)
            {
                return Err(PublicationError::Invalid);
            }
            previous = revision.key.number;
        }
        let mut previous: Option<&str> = None;
        for entry in &self.named {
            if crate::canonical_name(&entry.name).map_err(invalid)? != entry.name
                || previous.is_some_and(|name| name >= entry.name.as_str())
                || entry.revision.key.character != self.actor
                || entry.revision.key.mind != self.mind
                || entry.revision.key.number == 0
                || self
                    .base
                    .revision
                    .is_none_or(|n| entry.revision.key.number > n)
                || !numbers.insert(entry.revision.key.number)
            {
                return Err(PublicationError::Invalid);
            }
            previous = Some(&entry.name);
        }
        if self.base.revision.is_some_and(|n| !numbers.contains(&n)) {
            return Err(PublicationError::Invalid);
        }
        bounded(self, Self::MAX_BYTES)?;
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetiredDraftParent {
    pub(crate) format: u16,
    pub(crate) actor: ObjId,
    pub(crate) mind: u64,
    pub(crate) selection: DefinitionDigest,
    pub(crate) operation: CommitToken,
    pub(crate) base: EditBase,
    pub(crate) action: DefinitionRef,
}
impl Record for RetiredDraftParent {
    const FAMILY: &'static str = "roll_mind_retired_draft_parent";
    const MAX_BYTES: usize = 4096;
    fn key(&self) -> String {
        DraftParentSelection::key_for(self.actor, self.mind)
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1
            || self.mind == 0
            || self.base.revision == Some(0)
            || self.base.revision.is_some_and(|n| n > self.base.edit)
            || !super::known_action(&self.action).map_err(invalid)?
        {
            return Err(PublicationError::Invalid);
        }
        bounded(self, Self::MAX_BYTES)?;
        Ok(())
    }
}

// Enforce allocation bounds while decoding, before validating the enclosing row.
// The family byte cap alone is not an inventory-count/allocation contract.
fn inventory<'de, D, T, const N: usize>(decoder: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Bounded<T, const N: usize>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const N: usize> serde::de::Visitor<'de> for Bounded<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at most {N} retained entries")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            use serde::de::Error;
            if seq.size_hint().is_some_and(|n| n > N) {
                return Err(A::Error::custom("retained inventory exceeds bound"));
            }
            let mut rows = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(N));
            while rows.len() < N {
                match seq.next_element::<T>()? {
                    Some(row) => rows.push(row),
                    None => return Ok(rows),
                }
            }
            if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(A::Error::custom("retained inventory exceeds bound"));
            }
            Ok(rows)
        }
    }
    decoder.deserialize_seq(Bounded::<T, N>(std::marker::PhantomData))
}
fn unnamed_inventory<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<RevisionRef>, D::Error> {
    inventory::<D, RevisionRef, { crate::KEEP_UNNAMED }>(d)
}
fn named_inventory<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<NamedRetention>, D::Error> {
    inventory::<D, NamedRetention, { crate::MAX_NAMED_VERSIONS }>(d)
}

#[cfg(test)]
mod tests;
