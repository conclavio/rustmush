//! Permanent source obligation paired to a real native policy. Full native
//! documents stay with Economy/Tacsim; this bounded row names their exact
//! immutable consent and history without granting payment or departure.
use crate::commerce::{
    RevisionIdentity,
    ic_claims::{ClaimRecord, CloseKind},
    rating::RatingRef,
    records::{PublicationError, Record},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::{CommitToken, DefinitionDigest};
use mush_economy::managed_coverage::{
    CoverageConsentV1, CoverageDecisionKindV1, CoverageDecisionV1, CoverageHistory, CoverageOutput,
};
use mush_economy::{ContractId, HoldId};
use serde::{Deserialize, Serialize};

fn bad(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}
fn digest<T: Serialize>(
    domain: &str,
    value: &T,
    limit: usize,
) -> Result<DefinitionDigest, PublicationError> {
    DefinitionDigest::for_canonical_value_with_limit(
        domain,
        &serde_json::to_value(value).map_err(bad)?,
        limit,
    )
    .map_err(bad)
}
fn option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageRetention {
    pub(crate) format: u16,
    pub(crate) consent: CoverageConsentV1,
    pub(crate) revision: RevisionIdentity,
    pub(crate) rating: RatingRef,
    pub(crate) claim_digest: DefinitionDigest,
    pub(crate) contract: ContractId,
    pub(crate) hold: HoldId,
    pub(crate) binding_digest: DefinitionDigest,
    pub(crate) history_digest: DefinitionDigest,
    #[serde(deserialize_with = "option")]
    pub(crate) terminal: Option<CoverageDecisionV1>,
}
impl Record for CoverageRetention {
    const FAMILY: &'static str = "roll_mind_coverage";
    const MAX_BYTES: usize = 16384;
    fn key(&self) -> String {
        self.consent.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.consent.validate().map_err(bad)?;
        if self.format != 1
            || [self.claim_digest, self.binding_digest, self.history_digest]
                .iter()
                .any(|d| d.as_bytes() == &[0; 32])
        {
            return Err(PublicationError::Invalid);
        }
        if let Some(terminal) = &self.terminal {
            terminal.matches_consent(&self.consent).map_err(bad)?;
        }
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(bad)?,
            Self::MAX_BYTES,
        )
        .map_err(|_| PublicationError::Capacity)?;
        Ok(())
    }
}
impl CoverageRetention {
    pub fn operation(&self) -> CommitToken {
        self.consent.operation
    }
    pub fn consent(&self) -> &CoverageConsentV1 {
        &self.consent
    }
    pub fn revision(&self) -> RevisionIdentity {
        self.revision
    }
    pub fn rating(&self) -> RatingRef {
        self.rating
    }
    pub fn history_digest(&self) -> DefinitionDigest {
        self.history_digest
    }
    pub fn terminal(&self) -> Option<&CoverageDecisionV1> {
        self.terminal.as_ref()
    }
    pub fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        digest("roll.coverage.retention.v1", self, Self::MAX_BYTES)
    }
    pub(crate) fn root(&self) -> Result<RequiredRoot, PublicationError> {
        self.validate()?;
        // The root never changes at terminal. It retains the actual captured
        // executable even after accounts, terms, holders or certificates change.
        Ok(RequiredRoot {
            anchor: RootAnchor::Coverage(self.operation(), self.consent.digest().map_err(bad)?),
            identity: self.revision,
        })
    }
    pub fn matches_history(&self, history: &CoverageHistory) -> Result<(), PublicationError> {
        self.validate()?;
        if history.digest().map_err(bad)? != self.history_digest
            || history.consent() != &self.consent
            || history.contract_id() != self.contract
            || history.binding().hold() != self.hold
            || digest(
                "roll.coverage.native_binding.v1",
                history.binding(),
                mush_economy::managed_coverage::MAX_HISTORY_BYTES,
            )? != self.binding_digest
            || history.terminal().map(|t| t.decision()) != self.terminal.as_ref()
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    pub(crate) fn bind(
        output: &CoverageOutput,
        facts: &super::CoverageOpenFacts,
    ) -> Result<Self, PublicationError> {
        let history = output.history();
        let consent = history.consent();
        if history.terminal().is_some()
            || consent.basis.ic_open != facts.open()
            || consent.terms.buyer != facts.actor()
            || consent.basis.hull != facts.unit()
            || consent.basis.hull_title_revision != facts.hull_title_revision()
            || consent.basis.hull_revision != facts.hull_revision()
            || &consent.basis.hull_schema != facts.hull_schema()
            || consent.basis.seal_digest != facts.seal_digest()
            || consent.basis.admission_digest != facts.admission_digest()?
            || consent.terms.starts_at.clock != facts.observed_ic().clock
            || consent.terms.starts_at.at < facts.observed_ic().at
        {
            return Err(PublicationError::Mismatch);
        }
        let value = Self {
            format: 1,
            consent: consent.clone(),
            revision: facts.revision(),
            rating: facts.rating(),
            claim_digest: facts.claim().open.digest()?,
            contract: history.contract_id(),
            hold: history.binding().hold(),
            binding_digest: digest(
                "roll.coverage.native_binding.v1",
                history.binding(),
                mush_economy::managed_coverage::MAX_HISTORY_BYTES,
            )?,
            history_digest: history.digest().map_err(bad)?,
            terminal: None,
        };
        value.matches_history(history)?;
        Ok(value)
    }
    pub(crate) fn finish(
        &self,
        output: &CoverageOutput,
        claim: &ClaimRecord,
    ) -> Result<Self, PublicationError> {
        if self.terminal.is_some() {
            return Err(PublicationError::Mismatch);
        }
        let history = output.history();
        let terminal = history
            .terminal()
            .ok_or(PublicationError::Mismatch)?
            .decision();
        let mut after = self.clone();
        after.history_digest = history.digest().map_err(bad)?;
        after.terminal = Some(terminal.clone());
        after.matches_history(history)?;
        after.matches_claim(claim)?;
        Ok(after)
    }
    pub(crate) fn matches_claim(&self, claim: &ClaimRecord) -> Result<(), PublicationError> {
        self.validate()?;
        claim.validate()?;
        let open = &claim.open;
        if self.consent.basis.ic_open != open.operation
            || self.consent.basis.hull != open.unit
            || self.consent.terms.buyer != open.dispatcher
            || self.consent.basis.seal_digest != open.seal_digest
            || self.consent.basis.admission_digest != open.admission_digest()?
            || self.revision != open.revision
            || self.rating != open.rating
            || self.claim_digest != open.digest()?
        {
            return Err(PublicationError::Mismatch);
        }
        // A bound policy may await an actual Close or delayed payment. A Close
        // does not itself release liability, and current rights are irrelevant.
        let Some(decision) = &self.terminal else {
            return Ok(());
        };
        let close = claim.closed.as_ref().ok_or(PublicationError::Missing)?;
        let (operation, event, transition) = match &decision.reason {
            CoverageDecisionKindV1::Destroyed {
                loss,
                loss_digest,
                occurred_at,
                transition,
                ..
            } => {
                if !matches!(close.kind, CloseKind::Destroyed {}) || occurred_at != &close.at_ic {
                    return Err(PublicationError::Mismatch);
                }
                (*loss, *loss_digest, *transition)
            }
            CoverageDecisionKindV1::CleanClose {
                close,
                close_digest,
                transition,
            }
            | CoverageDecisionKindV1::Cancelled {
                close,
                close_digest,
                transition,
            }
            | CoverageDecisionKindV1::ExpiredClean {
                close,
                close_digest,
                transition,
            } => (*close, *close_digest, *transition),
        };
        if close.operation != operation
            || close.event_digest != event
            || close.transition != transition
            || (close.at_ic.clock == decision.at_ic.clock && close.at_ic.at > decision.at_ic.at)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
