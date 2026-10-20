//! Fixed exact references. No current manifest digest supplies historical truth.
use super::super::{FiledRevisionId, LicenceId, TermGeneration};
use super::*;
use mush_core::action::{ActionInvocation, PerceivedRef, PrincipalId};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{CommitToken, DefinitionRef, GOD, ModError};
use mush_economy::ExternalRef;
use serde::Deserialize;
use serde_json::json;

pub(super) const PROVIDER: &str = "minds";
pub(super) const FILED: &str = "filed";
pub(super) const LAPSED: &str = "licence.lapsed";
pub(super) const TRANSFERRED: &str = "licence.transferred";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FactKey {
    Filed(FiledRevisionId),
    Lapsed(LicenceId, TermGeneration),
    Transferred(CommitToken, u64),
}
impl FactKey {
    pub(super) fn parse(reference: &ExternalRef) -> Option<Self> {
        if reference.provider.as_str() != PROVIDER {
            return None;
        }
        match reference.kind.as_str() {
            FILED if reference.revision == Some(1) => {
                let id = FiledRevisionId::new(number(reference.id.as_str(), "filed")?).ok()?;
                (id.key() == reference.id.as_str()).then_some(Self::Filed(id))
            }
            LAPSED => {
                let id = LicenceId::new(number(reference.id.as_str(), "licence")?).ok()?;
                let generation = TermGeneration::new(reference.revision?).ok()?;
                (id.key() == reference.id.as_str()).then_some(Self::Lapsed(id, generation))
            }
            TRANSFERRED => {
                let operation = CommitToken::from_hex(reference.id.as_str()).ok()?;
                let generation = reference.revision?;
                (operation.to_hex() == reference.id.as_str()
                    && (2..=mush_economy::managed_paid_resale::MAX_HOLDER_GENERATION)
                        .contains(&generation))
                .then_some(Self::Transferred(operation, generation))
            }
            _ => None,
        }
    }
    pub(super) fn reference(self) -> ExternalRef {
        let (kind, id, revision) = match self {
            Self::Filed(id) => (FILED, id.key(), 1),
            Self::Lapsed(id, generation) => (LAPSED, id.key(), generation.get()),
            Self::Transferred(operation, generation) => {
                (TRANSFERRED, operation.to_hex(), generation)
            }
        };
        ExternalRef {
            provider: PROVIDER.into(),
            kind: kind.into(),
            id: id.into(),
            revision: Some(revision),
        }
    }
}
fn number(text: &str, prefix: &str) -> Option<u64> {
    let text = text.strip_prefix(prefix)?.strip_prefix('/')?;
    if text.len() != 16
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    u64::from_str_radix(text, 16).ok().filter(|v| *v != 0)
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn definitions() -> Result<[DefinitionDescriptor; 2], ModError> {
    let retention = super::super::authored_retirement::retention_reference()?;
    Ok([
        DefinitionDescriptor::new(DefinitionKind::new("action").map_err(error)?, DefinitionId::new("record_mind_term_lapse").map_err(error)?, 1,
            json!({"format":1,"protocol":"personal_mind_term_lapse_v1","authority":"actual_bounded_service_intent","subject":"accepted_term_generation","boundary":"grace_end","eligibility":"unchanged"}))
            .map_err(error)?.with_references(vec![retention]),
        DefinitionDescriptor::new(DefinitionKind::new("outcome").map_err(error)?, DefinitionId::new("mind_term_lapse_recorded").map_err(error)?, 1,
            json!({"format":1,"historical":true})).map_err(error)?,
    ])
}
pub(crate) fn action() -> Result<DefinitionRef, ModError> {
    let [action, _] = definitions()?;
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(error)?, action)
        .map_err(error)
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) licence: LicenceId,
    pub(super) generation: TermGeneration,
}
impl Request {
    pub(super) fn parse(
        inv: &ActionInvocation,
        actual: &DefinitionRef,
    ) -> Result<Self, PublicationError> {
        if actual != &action().map_err(invalid)?
            || inv.actor() != GOD
            || inv.principal() != &(PrincipalId::Object { id: GOD })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner.as_str() != crate::NS
        {
            return Err(PublicationError::Invalid);
        }
        serde_json::from_value(inv.parameters().clone()).map_err(invalid)
    }
    pub(super) fn key(&self) -> String {
        format!("{}/{}", self.licence.key(), self.generation.key())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fact_references_are_exact_canonical_subjects_and_generations() {
        let filed = FactKey::Filed(FiledRevisionId::new(7).unwrap());
        let lapse = FactKey::Lapsed(
            LicenceId::new(9).unwrap(),
            TermGeneration::new(u64::MAX).unwrap(),
        );
        let transfer = FactKey::Transferred(
            CommitToken::from_bytes([0x19; 16]),
            mush_economy::managed_paid_resale::MAX_HOLDER_GENERATION,
        );
        for key in [
            filed,
            lapse,
            transfer,
            FactKey::Transferred(CommitToken::from_bytes([0; 16]), 2),
        ] {
            assert_eq!(FactKey::parse(&key.reference()), Some(key));
        }
        for id in [
            "filed/7",
            "filed/0000000000000000",
            "filed/000000000000000A",
            "filed/0000000000000007/extra",
        ] {
            let mut bad = filed.reference();
            bad.id = id.into();
            assert!(FactKey::parse(&bad).is_none());
        }
        for revision in [None, Some(0), Some(2)] {
            let mut bad = filed.reference();
            bad.revision = revision;
            assert!(FactKey::parse(&bad).is_none());
        }
        for kind in ["seized", "unwarranted", "claimed", ""] {
            let mut bad = lapse.reference();
            bad.kind = kind.into();
            assert!(FactKey::parse(&bad).is_none());
        }
        for revision in [None, Some(0), Some(1), Some(u64::MAX)] {
            let mut bad = transfer.reference();
            bad.revision = revision;
            assert!(FactKey::parse(&bad).is_none());
        }
        for id in [
            "a".repeat(31),
            "A".repeat(32),
            "a".repeat(33),
            lapse.reference().id.to_string(),
        ] {
            let mut bad = transfer.reference();
            bad.id = id.into();
            assert!(FactKey::parse(&bad).is_none());
        }
        let mut bad = lapse.reference();
        bad.revision = Some(0);
        assert!(FactKey::parse(&bad).is_none());
        bad = lapse.reference();
        bad.provider = "roll".into();
        assert!(FactKey::parse(&bad).is_none());
    }
}
