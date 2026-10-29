//! Fixed exact references. No current manifest digest supplies historical truth.
use super::super::{FiledRevisionId, LicenceId, TermGeneration};
use super::*;
use mush_core::action::{ActionInvocation, PerceivedRef, PrincipalId};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{CommitToken, DefinitionRef, GOD, ModError, ObjId};
use mush_economy::ExternalRef;
use serde::Deserialize;
use serde_json::json;

pub(super) const PROVIDER: &str = "minds";
pub(super) const FILED: &str = "filed";
pub(super) const LAPSED: &str = "licence.lapsed";
pub(super) const TRANSFERRED: &str = "licence.transferred";
/// M8c B5/B7: the term generation flew on an ungoverned core
/// (`seal_marks`), scoped exactly as `licence.lapsed` is.
pub(super) const UNATTESTABLE: &str = "licence.unattestable";
/// Doc 17h cut M8d D6: a Registry seizure of the core in a hull (public: the hull named,
/// never the mind). Parsed here; its producer is the seizure, which M8d splits out.
pub(super) const SEIZED: &str = "seized";
/// Doc 17h cut M8d D6: a Registry inspection that found an ungoverned core. **Scoped** (fix
/// round K6, the lead's ruling; D-UM-42): a public fact keyed by hull would let anyone probe a
/// hull for an ungoverned core, so it resolves for no contract condition, as `claimed`; the
/// world's signal is the canon line, which names nobody, and the Red Dies clock.
pub(super) const UNWARRANTED: &str = "unwarranted";
/// Doc 17h cut M8d D6: reserved for M8g's claim (scoped). Parsed, never produced here.
pub(super) const CLAIMED: &str = "claimed";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FactKey {
    Filed(FiledRevisionId),
    Lapsed(LicenceId, TermGeneration),
    Transferred(CommitToken, u64),
    Unattestable(LicenceId, TermGeneration),
    /// `hull/<16 hex>` at revision = the hull's inspection sequence.
    Seized(ObjId, u64),
    /// `hull/<16 hex>` at revision = the hull's inspection sequence.
    Unwarranted(ObjId, u64),
    /// `hull/<16 hex>` at revision = the hull's inspection sequence (M8g).
    Claimed(ObjId, u64),
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
            UNATTESTABLE => {
                let id = LicenceId::new(number(reference.id.as_str(), "licence")?).ok()?;
                let generation = TermGeneration::new(reference.revision?).ok()?;
                (id.key() == reference.id.as_str()).then_some(Self::Unattestable(id, generation))
            }
            TRANSFERRED => {
                let operation = CommitToken::from_hex(reference.id.as_str()).ok()?;
                let generation = reference.revision?;
                (operation.to_hex() == reference.id.as_str()
                    && (2..=mush_economy::managed_paid_resale::MAX_HOLDER_GENERATION)
                        .contains(&generation))
                .then_some(Self::Transferred(operation, generation))
            }
            SEIZED | UNWARRANTED | CLAIMED => {
                let hull = number(reference.id.as_str(), "hull")?;
                let hull = ObjId(u32::try_from(hull).ok()?);
                let seq = reference.revision.filter(|seq| *seq != 0)?;
                (hull_key(hull) == reference.id.as_str()).then_some(match reference.kind.as_str() {
                    SEIZED => Self::Seized(hull, seq),
                    UNWARRANTED => Self::Unwarranted(hull, seq),
                    _ => Self::Claimed(hull, seq),
                })
            }
            _ => None,
        }
    }
    /// The public spellings: a filed revision and a seizure (a public act naming its hull).
    /// Every other kind — `unwarranted` (K6) included — malformed or not, stays scoped.
    pub(super) const fn is_public(self) -> bool {
        matches!(self, Self::Filed(_) | Self::Seized(..))
    }
    #[cfg(test)]
    pub(super) fn reference(self) -> ExternalRef {
        let (kind, id, revision) = match self {
            Self::Filed(id) => (FILED, id.key(), 1),
            Self::Lapsed(id, generation) => (LAPSED, id.key(), generation.get()),
            Self::Unattestable(id, generation) => (UNATTESTABLE, id.key(), generation.get()),
            Self::Transferred(operation, generation) => {
                (TRANSFERRED, operation.to_hex(), generation)
            }
            Self::Seized(hull, seq) => (SEIZED, hull_key(hull), seq),
            Self::Unwarranted(hull, seq) => (UNWARRANTED, hull_key(hull), seq),
            Self::Claimed(hull, seq) => (CLAIMED, hull_key(hull), seq),
        };
        ExternalRef {
            provider: PROVIDER.into(),
            kind: kind.into(),
            id: id.into(),
            revision: Some(revision),
        }
    }
}
/// A hull's fact id: `hull/` and its number as sixteen lower-case hex digits, the other
/// subjects' spelling. A fact reference is a contract's record, never a player surface.
pub(super) fn hull_key(hull: ObjId) -> String {
    format!("hull/{:016x}", hull.0)
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
        // M8c B7: `licence.unattestable` parses, scoped like the lapse.
        let unattestable =
            FactKey::Unattestable(LicenceId::new(9).unwrap(), TermGeneration::new(3).unwrap());
        for key in [
            filed,
            lapse,
            transfer,
            unattestable,
            FactKey::Transferred(CommitToken::from_bytes([0; 16]), 2),
        ] {
            assert_eq!(FactKey::parse(&key.reference()), Some(key));
        }
        assert_eq!(
            unattestable.reference().kind.as_str(),
            "licence.unattestable"
        );
        for revision in [None, Some(0)] {
            let mut bad = unattestable.reference();
            bad.revision = revision;
            assert!(FactKey::parse(&bad).is_none());
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
        // Doc 17h cut M8d D6 inverts the M8 pin: `seized`, `unwarranted` and `claimed` parse
        // over a hull and an inspection sequence; a licence id under their kinds, or no kind,
        // does not.
        for kind in ["seized", "unwarranted", "claimed", ""] {
            let mut bad = lapse.reference();
            bad.kind = kind.into();
            assert!(FactKey::parse(&bad).is_none());
        }
        let hull = ObjId(12);
        for key in [
            FactKey::Seized(hull, 1),
            FactKey::Unwarranted(hull, 3),
            FactKey::Claimed(hull, u64::MAX),
            FactKey::Unwarranted(ObjId(u32::MAX), 1),
        ] {
            assert_eq!(FactKey::parse(&key.reference()), Some(key));
        }
        assert!(FactKey::Seized(hull, 1).is_public());
        assert!(!FactKey::Unwarranted(hull, 1).is_public(), "K6: scoped");
        assert!(!FactKey::Claimed(hull, 1).is_public());
        assert!(!lapse.is_public() && filed.is_public());
        let found = FactKey::Unwarranted(hull, 2).reference();
        for revision in [None, Some(0)] {
            let mut bad = found.clone();
            bad.revision = revision;
            assert!(FactKey::parse(&bad).is_none());
        }
        for id in [
            "hull/12",
            "hull/000000000000000C",
            "hull/0000000100000000",
            "licence/000000000000000c",
            "hull/000000000000000c/extra",
        ] {
            let mut bad = found.clone();
            bad.id = id.into();
            assert!(FactKey::parse(&bad).is_none(), "{id}");
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
