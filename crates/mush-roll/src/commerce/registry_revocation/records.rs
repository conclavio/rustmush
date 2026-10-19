//! Immutable accepted term and cause; status is indexed by licence/generation,
//! never just the mutable licence pointer. Historical evidence grants no rights.
use super::super::{
    CharterId, FiledRevisionId, LineId, RevisionIdentity, TermGeneration, TermPhase, TermWindow,
    WorldIcSeconds,
    paid::PaidTermRecord,
    records::{OwnerTerm, RevisionPolicy},
};
use super::*;
use mush_core::{ClockStamp, DefinitionDigest};
use mush_org::SystemPrincipalId;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reason {
    RegistryReview,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AcceptedTerm {
    Owner { term: OwnerTerm },
    Paid { term: PaidTermRecord },
}
impl AcceptedTerm {
    pub(crate) fn licence(&self) -> LicenceId {
        match self {
            Self::Owner { term } => term.licence,
            Self::Paid { term } => term.licence(),
        }
    }
    pub(crate) fn generation(&self) -> TermGeneration {
        match self {
            Self::Owner { term } => term.generation,
            Self::Paid { term } => term.generation(),
        }
    }
    pub(crate) fn window(&self) -> TermWindow {
        match self {
            Self::Owner { term } => term.window,
            Self::Paid { term } => term.window(),
        }
    }
    pub(crate) fn operation(&self) -> CommitToken {
        match self {
            Self::Owner { term } => term.operation,
            Self::Paid { term } => term.operation(),
        }
    }
    pub(crate) fn policy(&self) -> RevisionPolicy {
        match self {
            Self::Owner { term } => term.revision,
            Self::Paid { term } => term.policy(),
        }
    }
    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        match self {
            Self::Owner { term } => term.validate(),
            Self::Paid { term } => term.validate(),
        }
    }
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "roll.registry_revocation.accepted_term.v1",
            &serde_json::to_value(self).map_err(bad)?,
        )
        .map_err(bad)
    }
    pub(crate) fn status_key(&self) -> String {
        RevokedTerm::key_for(self.licence(), self.generation())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "OperationWire")]
pub(crate) struct RevocationOperation {
    pub(crate) format: u16,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) authority: RegistryRevocationPolicy,
    pub(crate) actor: ObjId,
    pub(crate) authorization: authority::Authorization,
    pub(crate) term: AcceptedTerm,
    pub(crate) charter: CharterId,
    pub(crate) line: LineId,
    pub(crate) birth_operation: CommitToken,
    pub(crate) initial_operation: CommitToken,
    pub(crate) initial_issuer: SystemPrincipalId,
    pub(crate) filed: FiledRevisionId,
    pub(crate) revision: RevisionIdentity,
    pub(crate) reason: Reason,
    pub(crate) at_ic: ClockStamp,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationWire {
    format: u16,
    operation: CommitToken,
    action: DefinitionRef,
    authority: RegistryRevocationPolicy,
    actor: ObjId,
    authorization: authority::Authorization,
    term: AcceptedTerm,
    charter: CharterId,
    line: LineId,
    birth_operation: CommitToken,
    initial_operation: CommitToken,
    initial_issuer: SystemPrincipalId,
    filed: FiledRevisionId,
    revision: RevisionIdentity,
    reason: Reason,
    at_ic: ClockStamp,
}
impl TryFrom<OperationWire> for RevocationOperation {
    type Error = PublicationError;
    fn try_from(w: OperationWire) -> Result<Self, Self::Error> {
        let v = Self {
            format: w.format,
            operation: w.operation,
            action: w.action,
            authority: w.authority,
            actor: w.actor,
            authorization: w.authorization,
            term: w.term,
            charter: w.charter,
            line: w.line,
            birth_operation: w.birth_operation,
            initial_operation: w.initial_operation,
            initial_issuer: w.initial_issuer,
            filed: w.filed,
            revision: w.revision,
            reason: w.reason,
            at_ic: w.at_ic,
        };
        v.validate()?;
        Ok(v)
    }
}
fn envelope<R: Record + Serialize>(r: &R) -> Result<(), PublicationError> {
    let v = json!({"family":{"owner":crate::NS,"id":R::FAMILY},"schema":{"id":format!("{}.value.v1",R::FAMILY),"version":1},"key":r.key(),"value":r});
    mush_core::canonical_json_bytes_with_limit(&v, R::MAX_BYTES)
        .map_err(|_| PublicationError::Capacity)?;
    Ok(())
}
impl Record for RevocationOperation {
    const FAMILY: &'static str = "roll_registry_term_revocations";
    const MAX_BYTES: usize = 32768;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.authority.validate()?;
        self.authorization.validate()?;
        self.term.validate()?;
        let now = WorldIcSeconds::from_stamp(&self.at_ic).map_err(bad)?;
        if self.format != 1
            || self.action != self.authority.action().map_err(bad)?
            || self.authority.operators.binary_search(&self.actor).is_err()
            || self.authority.issuer != self.initial_issuer
            || self.operation == self.birth_operation
            || self.operation == self.initial_operation
            || self.operation == self.term.operation()
            || !matches!(
                self.term.window().phase_at(now),
                TermPhase::Active | TermPhase::Grace
            )
            || match self.term.policy() {
                RevisionPolicy::Pin { filed, revision } => {
                    filed != self.filed || revision != self.revision
                }
                RevisionPolicy::Follow { line } => line != self.line,
            }
        {
            return Err(PublicationError::Invalid);
        }
        envelope(self)
    }
}
impl RevocationOperation {
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "roll.registry_term_revocation.cause.v1",
            &serde_json::to_value(self).map_err(bad)?,
        )
        .map_err(bad)
    }
    pub(crate) fn root(&self) -> Result<RequiredRoot, PublicationError> {
        Ok(RequiredRoot {
            anchor: RootAnchor::RegistryRevocation(self.operation, self.digest()?),
            identity: self.revision,
        })
    }
    pub(crate) fn status(&self) -> Result<RevokedTerm, PublicationError> {
        self.validate()?;
        Ok(RevokedTerm {
            format: 1,
            licence: self.term.licence(),
            generation: self.term.generation(),
            operation: self.operation,
            term_digest: self.term.digest()?,
            window: self.term.window(),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StatusWire")]
pub(crate) struct RevokedTerm {
    pub(crate) format: u16,
    pub(crate) licence: LicenceId,
    pub(crate) generation: TermGeneration,
    pub(crate) operation: CommitToken,
    pub(crate) term_digest: DefinitionDigest,
    pub(crate) window: TermWindow,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusWire {
    format: u16,
    licence: LicenceId,
    generation: TermGeneration,
    operation: CommitToken,
    term_digest: DefinitionDigest,
    window: TermWindow,
}
impl TryFrom<StatusWire> for RevokedTerm {
    type Error = PublicationError;
    fn try_from(w: StatusWire) -> Result<Self, Self::Error> {
        let v = Self {
            format: w.format,
            licence: w.licence,
            generation: w.generation,
            operation: w.operation,
            term_digest: w.term_digest,
            window: w.window,
        };
        v.validate()?;
        Ok(v)
    }
}
impl Record for RevokedTerm {
    const FAMILY: &'static str = "roll_registry_revoked_terms";
    const MAX_BYTES: usize = 4096;
    fn key(&self) -> String {
        Self::key_for(self.licence, self.generation)
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1 {
            return Err(PublicationError::Invalid);
        }
        envelope(self)
    }
}
impl RevokedTerm {
    pub(crate) fn key_for(licence: LicenceId, generation: TermGeneration) -> String {
        format!("{}/{}", licence.key(), generation.key())
    }
    pub(crate) fn matches(&self, op: &RevocationOperation) -> Result<(), PublicationError> {
        self.validate()?;
        if self != &op.status()? {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    /// Only a new generation is allowed after the original deadline. This
    /// never restores source/use to the sanctioned predecessor, even on rollback.
    pub(crate) fn permits_successor(&self, now: WorldIcSeconds) -> bool {
        now >= self.window.grace_end()
    }
}
