//! Retained terminal data. Only the actual Tacsim host proof and registered
//! paired owner can authorize it; deserialization alone never pays or releases.
use super::*;
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, ObjId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoverageDecisionKindV1 {
    Destroyed {
        loss: CommitToken,
        loss_digest: DefinitionDigest,
        occurred_at: ClockStamp,
        zone: DefinitionRef,
        /// Actual immutable close/loss sequence, not wall-clock receipt order.
        transition: u64,
    },
    CleanClose {
        close: CommitToken,
        close_digest: DefinitionDigest,
        transition: u64,
    },
    Cancelled {
        close: CommitToken,
        close_digest: DefinitionDigest,
        transition: u64,
    },
    ExpiredClean {
        close: CommitToken,
        close_digest: DefinitionDigest,
        transition: u64,
    },
}
impl CoverageDecisionKindV1 {
    pub const fn pays(&self) -> bool {
        matches!(self, Self::Destroyed { .. })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CoverageDecisionV1 {
    pub format: u16,
    pub operation: CommitToken,
    pub action: DefinitionRef,
    pub actor: ObjId,
    pub at_ic: ClockStamp,
    pub posted_at_utc: i64,
    pub reason: CoverageDecisionKindV1,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    format: u16,
    operation: CommitToken,
    action: DefinitionRef,
    actor: ObjId,
    at_ic: ClockStamp,
    posted_at_utc: i64,
    reason: CoverageDecisionKindV1,
}
impl<'de> Deserialize<'de> for CoverageDecisionV1 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = Wire::deserialize(d)?;
        let v = Self {
            format: w.format,
            operation: w.operation,
            action: w.action,
            actor: w.actor,
            at_ic: w.at_ic,
            posted_at_utc: w.posted_at_utc,
            reason: w.reason,
        };
        v.validate().map_err(serde::de::Error::custom)?;
        Ok(v)
    }
}
impl CoverageDecisionV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1
            || self.actor.0 == 0
            || !matches!(self.at_ic.clock,ClockRef::WorldIc{generation} if generation!=0)
            || !action_matches(
                &self.action,
                if self.reason.pays() {
                    CLAIM_ACTION
                } else {
                    RELEASE_ACTION
                },
            )
        {
            return Err(refused());
        }
        let (evidence, digest, transition) = match &self.reason {
            CoverageDecisionKindV1::Destroyed {
                loss,
                loss_digest,
                occurred_at,
                zone,
                transition,
            } => {
                if occurred_at.clock != self.at_ic.clock
                    || occurred_at.at > self.at_ic.at
                    || zone.digest().as_bytes() == &[0; 32]
                {
                    return Err(refused());
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
            } => (close, close_digest, transition),
        };
        if *evidence == self.operation || digest.as_bytes() == &[0; 32] || *transition == 0 {
            return Err(refused());
        }
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(|_| refused())?,
            MAX_DECISION_BYTES,
        )
        .map_err(|_| refused())?;
        Ok(())
    }
    pub fn matches_consent(&self, c: &CoverageConsentV1) -> Result<(), ParticipantError> {
        self.validate()?;
        c.validate()?;
        if [c.operation, c.basis.product, c.basis.quote, c.basis.ic_open].contains(&self.operation)
            || self.posted_at_utc < c.accepted_at_utc
        {
            return Err(refused());
        }
        let cross_clock_cleanup = matches!(
            self.reason,
            CoverageDecisionKindV1::CleanClose { .. } | CoverageDecisionKindV1::Cancelled { .. }
        );
        if self.at_ic.clock == c.terms.starts_at.clock {
            if self.at_ic.at < c.terms.starts_at.at {
                return Err(refused());
            }
        } else if !cross_clock_cleanup {
            return Err(refused());
        }
        match &self.reason {
            CoverageDecisionKindV1::Destroyed {
                loss,
                occurred_at,
                zone,
                ..
            } => {
                if *loss == c.basis.ic_open
                    || occurred_at.clock != c.terms.starts_at.clock
                    || occurred_at.at < c.terms.starts_at.at
                    || occurred_at.at >= c.terms.ends_at.at
                    || zone != &c.basis.zone
                    || occurred_at
                        .at
                        .checked_add(c.terms.claim_delay_seconds)
                        .is_none_or(|due| self.at_ic.at < due)
                {
                    return Err(refused());
                }
            }
            CoverageDecisionKindV1::ExpiredClean { .. } => {
                if self.at_ic.at < c.terms.ends_at.at {
                    return Err(refused());
                }
            }
            CoverageDecisionKindV1::Cancelled { .. } => {
                if self.actor != c.terms.buyer {
                    return Err(refused());
                }
            }
            CoverageDecisionKindV1::CleanClose { .. } => (),
        }
        Ok(())
    }
}
