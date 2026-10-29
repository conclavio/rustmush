//! Stable identities for actual Moved callbacks. Every current entry is a
//! legacy post-mutation observer, including identified entries. This roster
//! exposes no callback, completion adapter, durable-ready flag or moving power.

use crate::ModError;
use crate::mechanics::{
    DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionRegistry, DefinitionValidationContext, OwnerId,
};
use serde::Serialize;
use serde_json::json;

pub const MAX_MOVED_OBSERVERS: usize = 16;
const ROSTER_BYTES: usize = 16 * 1024;
const KIND: &str = "moved_observer";

/// The only implemented callback delivery mode. Identifying a legacy observer
/// does not make its side effects safe for acknowledged movement completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MovedObserverMode {
    LegacyPostMutation,
}

/// One actual callback registration. No caller can construct or deserialize an
/// entry; `definition == None` means an anonymous legacy subscription.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MovedObserverEntry {
    owner: OwnerId,
    definition: Option<DefinitionRef>,
    mode: MovedObserverMode,
}
impl MovedObserverEntry {
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }
    pub const fn definition(&self) -> Option<&DefinitionRef> {
        self.definition.as_ref()
    }
    pub const fn mode(&self) -> MovedObserverMode {
        self.mode
    }
}

/// A bounded snapshot of the real registration order, with a digest over every
/// entry and its position. Inspection never executes observer code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovedObserverRoster {
    entries: Vec<MovedObserverEntry>,
    digest: DefinitionDigest,
}
impl MovedObserverRoster {
    pub fn entries(&self) -> &[MovedObserverEntry] {
        &self.entries
    }
    pub const fn digest(&self) -> DefinitionDigest {
        self.digest
    }
    /// Every present callback currently blocks prepared movement completion;
    /// none has an implemented durable completion adapter in this slice.
    pub fn has_blockers(&self) -> bool {
        !self.entries.is_empty()
    }
}

pub(crate) fn kind() -> Result<DefinitionKind, ModError> {
    DefinitionKind::new(KIND).map_err(bad)
}

pub(crate) fn descriptor(id: &str, version: u32) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        kind()?,
        DefinitionId::new(id).map_err(bad)?,
        version,
        json!({"format":1,"event":"moved","delivery":"legacy_post_mutation"}),
    )
    .map_err(bad)
}

pub(crate) fn validate_definition(ctx: &DefinitionValidationContext<'_>) -> Result<(), String> {
    let entry = ctx.definition();
    let reference = entry.reference();
    let expected = descriptor(reference.key().id.as_str(), reference.version())
        .map_err(|error| error.to_string())?;
    if reference.key().kind != *expected.kind()
        || entry.payload() != expected.payload()
        || !entry.references().is_empty()
    {
        return Err("Moved observer does not match the exact Core legacy schema".into());
    }
    let expected = crate::DefinitionRegistryBuilder::new()
        .register(reference.key().owner.clone(), expected)
        .map_err(|error| error.to_string())?;
    if reference != &expected {
        return Err("Moved observer exact definition identity differs".into());
    }
    Ok(())
}

pub(crate) fn roster<'a>(
    actual: impl Iterator<Item = (&'a str, Option<&'a DefinitionRef>)>,
    definitions: Option<&DefinitionRegistry>,
) -> Result<MovedObserverRoster, ModError> {
    let mut entries = Vec::new();
    for (owner, definition) in actual {
        if entries.len() == MAX_MOVED_OBSERVERS {
            return Err(bad("Moved observer roster exceeds its bound"));
        }
        let owner = OwnerId::parse_canonical(owner).map_err(bad)?;
        if let Some(reference) = definition {
            if reference.key().owner != owner || reference.key().kind.as_str() != KIND {
                return Err(bad("Moved observer owner or kind differs"));
            }
            let definitions =
                definitions.ok_or_else(|| bad("Moved observer definitions are not frozen"))?;
            let entry = definitions.inspect_exact(reference).map_err(bad)?;
            if definitions.current(reference.key()) != Some(reference)
                || entry.payload()
                    != descriptor(reference.key().id.as_str(), reference.version())?.payload()
                || !entry.references().is_empty()
            {
                return Err(bad(
                    "Moved observer callback is not the exact current legacy definition",
                ));
            }
        }
        entries.push(MovedObserverEntry {
            owner,
            definition: definition.cloned(),
            mode: MovedObserverMode::LegacyPostMutation,
        });
    }
    let digest = DefinitionDigest::for_canonical_value_with_limit(
        "core.moved_observer_roster.v1",
        &serde_json::to_value(&entries).map_err(bad)?,
        ROSTER_BYTES,
    )
    .map_err(bad)?;
    Ok(MovedObserverRoster { entries, digest })
}

fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

#[cfg(test)]
mod tests;
