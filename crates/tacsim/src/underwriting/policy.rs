//! Exact policy/native/host pair data. Only the registered producer and actual
//! prepared owner outputs publish these rows; decoding supplies no authority.
use super::{
    CoverageError,
    protocol::Protocol,
    records::{self, Quote},
};
use crate::minds::ic_sortie::CommittedCoverageClose;
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId, World};
use mush_economy::managed_coverage::{CoverageConsentV1, CoverageDecisionKindV1, CoverageHistory};
use mush_economy::{AccountId, Amount, InstrumentId, TransactionId};
use mush_roll::commerce::ic_claims::IcCloseKind;
use serde::{Deserialize, Serialize};

pub(super) const POLICY_BYTES: usize = 81920;
pub(super) const EXPOSURE_BYTES: usize = 4096;
pub(super) const HULL_BYTES: usize = 4096;
pub(super) const TERMINAL_BYTES: usize = 16384;
pub(super) const RECEIPT_BYTES: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Policy {
    pub(super) format: u16,
    pub(super) request: DefinitionDigest,
    pub(super) operation: CommitToken,
    pub(super) action: DefinitionRef,
    pub(super) protocol: Protocol,
    /// Monotone per-hull chain, retained permanently even after terminal release.
    pub(super) sequence: u64,
    #[serde(deserialize_with = "records::option")]
    pub(super) previous: Option<CommitToken>,
    pub(super) quote: CommitToken,
    pub(super) quote_digest: DefinitionDigest,
    pub(super) history: CoverageHistory,
}
impl Policy {
    pub(super) fn validate(&self) -> Result<(), CoverageError> {
        self.protocol.validate()?;
        self.history.digest().map_err(|_| CoverageError::Product)?;
        let c = self.history.consent();
        if self.format != 1
            || self.request.as_bytes() == &[0; 32]
            || self.sequence == 0
            || (self.sequence == 1) != self.previous.is_none()
            || self.previous == Some(self.operation)
            || self.action != self.protocol.action(2)?
            || c.operation != self.operation
            || c.effect != self.action
            || c.basis.quote != self.quote
            || c.basis.quote_digest != self.quote_digest
        {
            return Err(CoverageError::Product);
        }
        records::value(self, POLICY_BYTES)?;
        Ok(())
    }
    pub(super) fn decode(raw: &[u8]) -> Result<Self, CoverageError> {
        let row: Self = records::decode(raw, POLICY_BYTES)?;
        row.validate()?;
        Ok(row)
    }
    pub(super) fn digest(&self) -> Result<DefinitionDigest, CoverageError> {
        self.validate()?;
        records::digest("tacsim.coverage.policy.v1", self, POLICY_BYTES)
    }
    pub(super) fn matches_quote(&self, q: &Quote) -> Result<(), CoverageError> {
        self.validate()?;
        q.validate()?;
        let c = self.history.consent();
        if self.quote != q.operation
            || self.quote_digest != q.digest()?
            || self.protocol != q.protocol
            || c.basis != q.native_basis()?
            || c.terms
                != q.terms
                    .bind_window(c.terms.starts_at.clone())
                    .map_err(|_| CoverageError::Terms)?
        {
            return Err(CoverageError::Product);
        }
        Ok(())
    }
    pub(super) fn terminal_of(&self, before: &Self) -> Result<(), CoverageError> {
        self.validate()?;
        before.validate()?;
        if before.history.terminal().is_some()
            || self.history.terminal().is_none()
            || self.request != before.request
            || self.format != before.format
            || self.operation != before.operation
            || self.action != before.action
            || self.protocol != before.protocol
            || self.sequence != before.sequence
            || self.previous != before.previous
            || self.quote != before.quote
            || self.quote_digest != before.quote_digest
            || self.history.consent() != before.history.consent()
            || self.history.contract_id() != before.history.contract_id()
            || self.history.binding() != before.history.binding()
        {
            return Err(CoverageError::Product);
        }
        Ok(())
    }
}

/// This captures only the actual audited opaque host readback. The same record
/// is joined again at cold recovery; its fields alone never authorize payment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CloseProof {
    pub(super) open: CommitToken,
    pub(super) close: CommitToken,
    pub(super) digest: DefinitionDigest,
    pub(super) transition: u64,
    pub(super) at_ic: ClockStamp,
    pub(super) zone: DefinitionRef,
    pub(super) kind: IcCloseKind,
}
impl CloseProof {
    pub(super) fn capture(
        world: &World,
        proof: &CommittedCoverageClose,
    ) -> Result<Self, CoverageError> {
        proof.recheck(world).map_err(|_| CoverageError::Authority)?;
        let result = Self {
            open: proof.open(),
            close: proof.operation().map_err(|_| CoverageError::Authority)?,
            digest: proof.digest().map_err(|_| CoverageError::Authority)?,
            transition: proof.transition().map_err(|_| CoverageError::Authority)?,
            at_ic: proof.at_ic().map_err(|_| CoverageError::Authority)?.clone(),
            zone: proof.zone().map_err(|_| CoverageError::Authority)?.clone(),
            kind: proof.kind().map_err(|_| CoverageError::Authority)?,
        };
        result.validate()?;
        Ok(result)
    }
    fn validate(&self) -> Result<(), CoverageError> {
        if self.open == self.close
            || self.transition == 0
            || self.digest.as_bytes() == &[0; 32]
            || self.zone.digest().as_bytes() == &[0; 32]
            || !matches!(self.at_ic.clock,ClockRef::WorldIc{generation}if generation!=0)
        {
            return Err(CoverageError::Product);
        }
        records::value(self, 4096)?;
        Ok(())
    }
    /// Coverage is evaluated against immutable accepted time and zone. A later
    /// clock reset or a delayed claim cannot turn a covered loss into a release.
    pub(super) fn covered_loss(&self, c: &CoverageConsentV1) -> Result<bool, CoverageError> {
        self.validate()?;
        c.validate().map_err(|_| CoverageError::Product)?;
        if self.open != c.basis.ic_open {
            return Err(CoverageError::Product);
        }
        Ok(matches!(self.kind, IcCloseKind::Destroyed)
            && self.zone == c.basis.zone
            && self.at_ic.clock == c.terms.starts_at.clock
            && self.at_ic.at >= c.terms.starts_at.at
            && self.at_ic.at < c.terms.ends_at.at)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Terminal {
    pub(super) format: u16,
    pub(super) request: DefinitionDigest,
    pub(super) operation: CommitToken,
    pub(super) action: DefinitionRef,
    pub(super) protocol: Protocol,
    pub(super) policy: CommitToken,
    pub(super) policy_digest: DefinitionDigest,
    pub(super) native_history: DefinitionDigest,
    pub(super) close: CloseProof,
}
impl Terminal {
    pub(super) fn validate(&self) -> Result<(), CoverageError> {
        self.protocol.validate()?;
        self.close.validate()?;
        if self.format != 1
            || self.request.as_bytes() == &[0; 32]
            || self.operation == self.policy
            || self.operation == self.close.close
            || self.operation == self.close.open
            || self.policy_digest.as_bytes() == &[0; 32]
            || self.native_history.as_bytes() == &[0; 32]
            || ![self.protocol.action(3)?, self.protocol.action(4)?].contains(&self.action)
        {
            return Err(CoverageError::Product);
        }
        records::value(self, TERMINAL_BYTES)?;
        Ok(())
    }
    pub(super) fn decode(raw: &[u8]) -> Result<Self, CoverageError> {
        let row: Self = records::decode(raw, TERMINAL_BYTES)?;
        row.validate()?;
        Ok(row)
    }
    pub(super) fn matches_policy(&self, p: &Policy) -> Result<(), CoverageError> {
        self.validate()?;
        p.validate()?;
        let native = p.history.terminal().ok_or(CoverageError::Product)?;
        let d = native.decision();
        let c = p.history.consent();
        if self.policy != p.operation
            || self.policy_digest != p.digest()?
            || self.native_history != p.history.digest().map_err(|_| CoverageError::Product)?
            || self.operation != d.operation
            || self.action != d.action
            || self.action != self.protocol.action(if d.reason.pays() { 3 } else { 4 })?
            || self.close.open != c.basis.ic_open
            || (self.close.at_ic.clock == d.at_ic.clock && self.close.at_ic.at > d.at_ic.at)
        {
            return Err(CoverageError::Product);
        }
        let (event, digest, transition) = match &d.reason {
            CoverageDecisionKindV1::Destroyed {
                loss,
                loss_digest,
                occurred_at,
                zone,
                transition,
            } => {
                if !self.close.covered_loss(c)?
                    || occurred_at != &self.close.at_ic
                    || zone != &self.close.zone
                {
                    return Err(CoverageError::Product);
                }
                (loss, loss_digest, transition)
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
            } => {
                if self.close.covered_loss(c)? {
                    return Err(CoverageError::Product);
                }
                (close, close_digest, transition)
            }
        };
        if *event != self.close.close
            || *digest != self.close.digest
            || *transition != self.close.transition
        {
            return Err(CoverageError::Product);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ExposureState {
    Held {},
    Paid { terminal: CommitToken },
    Released { terminal: CommitToken },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Exposure {
    pub(super) format: u16,
    pub(super) policy: CommitToken,
    pub(super) hull: ObjId,
    pub(super) treasury: AccountId,
    pub(super) instrument: InstrumentId,
    pub(super) liability: Amount,
    pub(super) history: DefinitionDigest,
    pub(super) state: ExposureState,
}
impl Exposure {
    pub(super) fn from_policy(p: &Policy) -> Result<Self, CoverageError> {
        p.validate()?;
        let c = p.history.consent();
        let state = match p.history.terminal() {
            None => ExposureState::Held {},
            Some(t) if t.decision().reason.pays() => ExposureState::Paid {
                terminal: t.decision().operation,
            },
            Some(t) => ExposureState::Released {
                terminal: t.decision().operation,
            },
        };
        let row = Self {
            format: 1,
            policy: p.operation,
            hull: c.basis.hull,
            treasury: c.terms.treasury,
            instrument: c.terms.instrument.clone(),
            liability: c.terms.liability,
            history: p.history.digest().map_err(|_| CoverageError::Product)?,
            state,
        };
        records::value(&row, EXPOSURE_BYTES)?;
        Ok(row)
    }
    pub(super) fn decode(raw: &[u8]) -> Result<Self, CoverageError> {
        let row: Self = records::decode(raw, EXPOSURE_BYTES)?;
        if row.format != 1
            || row.hull.0 == 0
            || row.liability.minor() <= 0
            || row.instrument.as_str().len() > 128
            || row.history.as_bytes() == &[0; 32]
            || matches!(row.state,ExposureState::Paid{terminal}|ExposureState::Released{terminal}if terminal==row.policy)
        {
            return Err(CoverageError::Product);
        }
        Ok(row)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HullHead {
    pub(super) format: u16,
    pub(super) hull: ObjId,
    pub(super) sequence: u64,
    pub(super) latest: CommitToken,
    pub(super) policy_digest: DefinitionDigest,
    #[serde(deserialize_with = "records::option")]
    pub(super) active: Option<CommitToken>,
}
impl HullHead {
    pub(super) fn from_policy(p: &Policy) -> Result<Self, CoverageError> {
        p.validate()?;
        let row = Self {
            format: 1,
            hull: p.history.consent().basis.hull,
            sequence: p.sequence,
            latest: p.operation,
            policy_digest: p.digest()?,
            active: p.history.terminal().is_none().then_some(p.operation),
        };
        records::value(&row, HULL_BYTES)?;
        Ok(row)
    }
    pub(super) fn decode(raw: &[u8]) -> Result<Self, CoverageError> {
        let row: Self = records::decode(raw, HULL_BYTES)?;
        if row.format != 1
            || row.hull.0 == 0
            || row.sequence == 0
            || row.policy_digest.as_bytes() == &[0; 32]
            || row.active.is_some_and(|v| v != row.latest)
        {
            return Err(CoverageError::Product);
        }
        Ok(row)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    pub(super) format: u16,
    pub(super) operation: CommitToken,
    pub(super) policy: CommitToken,
    pub(super) history: DefinitionDigest,
    pub(super) transaction: TransactionId,
    pub(super) journal_seq: u64,
    pub(super) state: ExposureState,
}
impl Receipt {
    pub(super) fn from_policy(p: &Policy) -> Result<Self, CoverageError> {
        let t = p.history.terminal().ok_or(CoverageError::Product)?;
        let row = Self {
            format: 1,
            operation: t.decision().operation,
            policy: p.operation,
            history: p.history.digest().map_err(|_| CoverageError::Product)?,
            transaction: t.transaction(),
            journal_seq: t.journal_seq(),
            state: Exposure::from_policy(p)?.state,
        };
        records::value(&row, RECEIPT_BYTES)?;
        Ok(row)
    }
    pub(super) fn decode(raw: &[u8]) -> Result<Self, CoverageError> {
        let row: Self = records::decode(raw, RECEIPT_BYTES)?;
        if row.format != 1
            || row.operation == row.policy
            || row.history.as_bytes() == &[0; 32]
            || !matches!(row.state,ExposureState::Paid{terminal}|ExposureState::Released{terminal}if terminal==row.operation)
        {
            return Err(CoverageError::Product);
        }
        Ok(row)
    }
}
