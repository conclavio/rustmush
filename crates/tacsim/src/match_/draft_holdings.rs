//! Character-owned saved revisions resolved from the workflow owner.
//! No working draft or mutable head is used after this payload is returned.

use crate::arena::core::CoreClass;
use crate::arena::profile_seal::MindOrigin;
use crate::arena::runtime_card::{CardDefinition, ProfileDefinition};
use crate::arena::schema::validate_envelope;
use mush_core::{ObjId, World};
use mush_roll::{Revision, RevisionRef, RollError, RollStore, SourceAdapter};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::sync::Arc;

/// Fully resolved source and executable envelope for an owned saved revision.
/// A checkout retains this resolved clone; preview may resolve Follow again.
/// Later edits never alter a clone that has already been sealed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedMind {
    pub name: SmolStr,
    pub origin: MindOrigin,
    pub source_sha256: [u8; 32],
    pub source: Arc<str>,
    pub abi: SmolStr,
    pub rated: CoreClass,
    pub definition: Arc<CardDefinition>,
}

/// Stable lobby/character selection. A name is display-only after resolution.
/// Follow is re-previewed for admission and resolved exactly once at checkout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedChoice {
    pub character: ObjId,
    pub mind: u64,
    pub selection: mush_roll::Selection,
}
impl SavedChoice {
    /// Build from an authenticated character and owned name, never a request's
    /// arbitrary owner. None pin means Follow this stable mind id.
    /// # Errors
    /// Missing/cross-character/cross-mind pin or malformed digest identity.
    pub fn choose(
        owner: &RollStore,
        actor: ObjId,
        name: &str,
        pin: Option<RevisionRef>,
    ) -> Result<Self, RollError> {
        let mind = owner.mind(actor, name)?;
        let selection = match pin {
            Some(revision) if revision.key.character == actor && revision.key.mind == mind.id() => {
                if owner.revision(actor, revision.key)?.reference() != revision {
                    return Err(RollError::Missing);
                }
                mush_roll::Selection::Pin { revision }
            }
            Some(_) => return Err(RollError::Missing),
            None => mush_roll::Selection::Follow { mind: mind.id() },
        };
        Ok(Self {
            character: actor,
            mind: mind.id(),
            selection,
        })
    }
    /// Shared preview/checkout resolution. Checkout immediately freezes the
    /// result into custody and creates its retained exact revision reference.
    /// # Errors
    /// Lost ownership/readability, unresolved saved head, source or metadata.
    pub fn resolve<S: SourceAdapter>(
        &self,
        owner: &RollStore,
        sources: &S,
        actor: ObjId,
    ) -> Result<SavedMind, RollError> {
        if actor != self.character {
            return Err(RollError::Missing);
        }
        let mind = owner.owned_mind(actor, self.mind)?;
        let reference = match self.selection {
            mush_roll::Selection::Pin { revision }
                if revision.key.character == actor && revision.key.mind == self.mind =>
            {
                revision
            }
            mush_roll::Selection::Follow { mind: id } if id == self.mind => owner
                .revision(actor, mind.latest().ok_or(RollError::Missing)?)?
                .reference(),
            _ => return Err(RollError::Missing),
        };
        let revision = owner.revision(actor, reference.key)?;
        if revision.reference() != reference {
            return Err(RollError::Missing);
        }
        SavedMind::from_revision(sources, actor, mind.name(), revision)
    }
}

impl SavedMind {
    /// Revalidate immutable source identity and metadata before admitting a saved
    /// revision. Saving/boot already compiled it; listing must not compile again.
    /// # Errors
    /// Ownership, missing/quarantined source, ABI, digest or metadata mismatch.
    pub fn resolve<S: SourceAdapter>(
        owner: &RollStore,
        sources: &S,
        actor: ObjId,
        name: &str,
        pin: Option<RevisionRef>,
    ) -> Result<Self, RollError> {
        let mind = owner.mind(actor, name)?;
        let key = match pin {
            Some(pin) if pin.key.character == actor && pin.key.mind == mind.id() => pin.key,
            Some(_) => return Err(RollError::Missing),
            None => mind.latest().ok_or(RollError::Missing)?,
        };
        let revision = owner.revision(actor, key)?;
        if pin.is_some_and(|pin| revision.reference() != pin) {
            return Err(RollError::Missing);
        }
        Self::from_revision(sources, actor, mind.name(), revision)
    }

    fn from_revision<S: SourceAdapter>(
        sources: &S,
        actor: ObjId,
        name: &str,
        revision: &Revision,
    ) -> Result<Self, RollError> {
        let reference = revision.reference();
        if reference.key.character != actor {
            return Err(RollError::Missing);
        }
        revision.verify_source(sources)?;
        let envelope = revision.envelope();
        if envelope.mind_abi != crate::minds::MIND_ABI {
            return Err(invalid(
                "mind_abi",
                "saved revision has another executable ABI",
            ));
        }
        let rated = CoreClass::parse(&envelope.declared_class)
            .filter(|c| *c != CoreClass::Zero)
            .ok_or_else(|| invalid("declared_class", "authored source requires I, II or III"))?;
        let decoded = validate_envelope(
            &envelope.schema,
            &envelope.presets,
            &envelope.profiles,
            &envelope.rules,
            rated,
        )
        .map_err(|e| invalid(&e.field, e.message))?;
        let definition = CardDefinition {
            schema: decoded.schema,
            declared_class: rated,
            profiles: decoded
                .profiles
                .into_iter()
                .map(|(name, values)| ProfileDefinition {
                    rules: decoded.rules.get(&name).cloned().unwrap_or_default(),
                    name,
                    values,
                })
                .collect(),
        };
        definition
            .validate()
            .map_err(|e| invalid(&e.field, e.message))?;
        Ok(Self {
            name: SmolStr::new(name),
            origin: MindOrigin::Draft {
                character: actor,
                mind: reference.key.mind,
                revision: reference.key.number,
                digest: reference.digest,
            },
            source_sha256: *revision.source().as_bytes(),
            source: Arc::from(sources.read_source(revision.source())?),
            abi: SmolStr::new(&envelope.mind_abi),
            rated,
            definition: Arc::new(definition),
        })
    }

    /// Recheck a selected saved pin at checkout under its original character.
    /// A removed library entry/quarantined revision refuses; a changed head does
    /// not replace the selected executable. Native ROM substitution is separate.
    /// # Errors
    /// The original exact saved revision is no longer readable or differs.
    pub fn revalidate<S: SourceAdapter>(
        &self,
        owner: &RollStore,
        sources: &S,
        actor: ObjId,
    ) -> Result<(), RollError> {
        let MindOrigin::Draft {
            character,
            mind,
            revision,
            digest,
        } = &self.origin
        else {
            return Err(RollError::Missing);
        };
        if *character != actor {
            return Err(RollError::Missing);
        }
        let pin = RevisionRef {
            key: mush_roll::RevisionKey {
                character: *character,
                mind: *mind,
                number: *revision,
            },
            digest: *digest,
        };
        owner.owned_mind(actor, *mind)?;
        let revision = owner.revision(actor, pin.key)?;
        if revision.reference() != pin {
            return Err(RollError::Missing);
        }
        let current = Self::from_revision(sources, actor, &self.name, revision)?;
        if current != *self {
            return Err(invalid("selection", "saved executable changed"));
        }
        Ok(())
    }
}

/// List only this authenticated character's valid saved heads, in owner order.
/// No account-level alias grants access to another character's library.
#[must_use]
pub fn saved_by(w: &World, character: ObjId) -> Vec<SavedMind> {
    let (Some(owner), Some(sources)) = (
        w.module_state::<RollStore>(),
        w.module_state::<crate::TacsimStore>(),
    ) else {
        return Vec::new();
    };
    owner
        .names(character)
        .into_iter()
        .filter_map(|name| SavedMind::resolve(owner, sources, character, name, None).ok())
        .collect()
}
fn invalid(field: &str, message: impl Into<String>) -> RollError {
    RollError::Invalid {
        field: field.into(),
        message: message.into(),
    }
}
