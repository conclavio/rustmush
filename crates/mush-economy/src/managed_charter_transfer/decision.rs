//! Closed retained terminal evidence. These values are data, never a service
//! capability or Registry authority; current proof is repeated by both owners.
use super::*;
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, ObjId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CharterPendingDecisionKindV1 {
    LastClaimClosed {
        next_transition: u64,
    },
    DeadlineReached {
        next_transition: u64,
        open_owned: u32,
    },
    RegistryRevoked {
        sanction: CommitToken,
        licence: u64,
        generation: u64,
        term_digest: DefinitionDigest,
    },
    ClockChanged,
    RecipientUnavailable,
}
impl CharterPendingDecisionKindV1 {
    pub fn settles(&self) -> bool {
        matches!(
            self,
            Self::LastClaimClosed { .. } | Self::DeadlineReached { .. }
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CharterPendingDecisionV1 {
    pub format: u16,
    pub operation: CommitToken,
    pub action: DefinitionRef,
    pub actor: ObjId,
    pub at_ic: ClockStamp,
    pub posted_at_utc: i64,
    pub reason: CharterPendingDecisionKindV1,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionWire {
    format: u16,
    operation: CommitToken,
    action: DefinitionRef,
    actor: ObjId,
    at_ic: ClockStamp,
    posted_at_utc: i64,
    reason: CharterPendingDecisionKindV1,
}
impl<'de> Deserialize<'de> for CharterPendingDecisionV1 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = DecisionWire::deserialize(d)?;
        let row = Self {
            format: w.format,
            operation: w.operation,
            action: w.action,
            actor: w.actor,
            at_ic: w.at_ic,
            posted_at_utc: w.posted_at_utc,
            reason: w.reason,
        };
        row.validate().map_err(serde::de::Error::custom)?;
        Ok(row)
    }
}
impl CharterPendingDecisionV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1
            || self.actor.0 == 0
            || !matches!(self.at_ic.clock,ClockRef::WorldIc{generation} if generation!=0)
        {
            return Err(refused());
        }
        match &self.reason {
            CharterPendingDecisionKindV1::LastClaimClosed { next_transition }
            | CharterPendingDecisionKindV1::DeadlineReached {
                next_transition, ..
            } => {
                if *next_transition == 0 || !action_matches(&self.action, SETTLE_ACTION, 1) {
                    return Err(refused());
                }
            }
            CharterPendingDecisionKindV1::RegistryRevoked {
                sanction,
                licence,
                generation,
                ..
            } => {
                // The same-union Registry path may use its exact policy-specific
                // action. The configured owner boundary must compare the entire
                // reference, never this structural locator alone.
                if *licence == 0
                    || *generation == 0
                    || (!action_matches(&self.action, REFUND_ACTION, 1)
                        && !action_matches(&self.action, "registry_term_revocation", 1))
                    || (action_matches(&self.action, "registry_term_revocation", 1)
                        != (*sanction == self.operation))
                {
                    return Err(refused());
                }
            }
            _ => {
                if !action_matches(&self.action, REFUND_ACTION, 1) {
                    return Err(refused());
                }
            }
        }
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(|_| ParticipantError::Encoding)?,
            2048,
        )
        .map_err(|_| refused())?;
        Ok(())
    }
    pub fn matches_consent(
        &self,
        consent: &CharterPendingConsentV1,
    ) -> Result<(), ParticipantError> {
        self.validate()?;
        consent.validate()?;
        if self.operation == consent.operation
            || self.operation == consent.terms.offer
            || self.operation == consent.terms.birth
            || self.operation == consent.terms.ownership.operation
            || self.operation == consent.terms.owner_term.operation
            || self.posted_at_utc < consent.accepted_at_utc
        {
            return Err(refused());
        }
        match &self.reason {
            CharterPendingDecisionKindV1::ClockChanged => {
                if self.at_ic.clock == consent.accepted_at_ic.clock {
                    return Err(refused());
                }
            }
            CharterPendingDecisionKindV1::DeadlineReached { .. } => {
                if !consent.deadline_reached(&self.at_ic)? {
                    return Err(refused());
                }
            }
            CharterPendingDecisionKindV1::RegistryRevoked {
                licence,
                generation,
                term_digest,
                ..
            } => {
                let term = &consent.terms.owner_term;
                if *licence != term.licence
                    || *generation != term.generation
                    || *term_digest != term.digest
                {
                    return Err(refused());
                }
                // Real exact sanction proof determines the captured generation;
                // cleanup does not borrow current parent, buyer or owner rights.
            }
            CharterPendingDecisionKindV1::LastClaimClosed { .. }
            | CharterPendingDecisionKindV1::RecipientUnavailable => {
                if self.at_ic.clock != consent.accepted_at_ic.clock
                    || self.at_ic.at < consent.accepted_at_ic.at
                {
                    return Err(refused());
                }
            }
        }
        Ok(())
    }
}
