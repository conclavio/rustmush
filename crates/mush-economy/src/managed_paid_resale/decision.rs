//! Immutable terminal observation. This is not the private service intent or
//! live Registry authorization, both of which the paired owners must retain.
use super::*;
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, ObjId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PaidResaleDecisionKindV1 {
    LastClaimClosed {
        next_transition: u64,
    },
    DeadlineReached {
        next_transition: u64,
        open_paid: u32,
    },
    RegistryRevoked {
        sanction: CommitToken,
        licence: u64,
        generation: u64,
        term_digest: DefinitionDigest,
    },
    ClockChanged {},
    RecipientUnavailable {},
    MonetaryBindingChanged {},
}
impl PaidResaleDecisionKindV1 {
    pub fn settles(&self) -> bool {
        matches!(
            self,
            Self::LastClaimClosed { .. } | Self::DeadlineReached { .. }
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PaidResaleDecisionV1 {
    pub format: u16,
    pub operation: CommitToken,
    pub action: DefinitionRef,
    pub actor: ObjId,
    pub at_ic: ClockStamp,
    pub posted_at_utc: i64,
    pub reason: PaidResaleDecisionKindV1,
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
    reason: PaidResaleDecisionKindV1,
}
impl<'de> Deserialize<'de> for PaidResaleDecisionV1 {
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
impl PaidResaleDecisionV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        subject::world(&self.at_ic.clock)?;
        if self.format != 1 || self.actor.0 == 0 {
            return Err(refused());
        }
        match &self.reason {
            PaidResaleDecisionKindV1::LastClaimClosed { next_transition } => {
                if *next_transition == 0 || !action_matches(&self.action, SETTLE) {
                    return Err(refused());
                }
            }
            PaidResaleDecisionKindV1::DeadlineReached {
                next_transition,
                open_paid,
            } => {
                if *next_transition == 0
                    || *open_paid > 32768
                    || !action_matches(&self.action, SETTLE)
                {
                    return Err(refused());
                }
            }
            PaidResaleDecisionKindV1::RegistryRevoked {
                sanction,
                licence,
                generation,
                ..
            } => {
                if *licence == 0
                    || *generation == 0
                    || (!action_matches(&self.action, REFUND) && !registry_action(&self.action))
                    || (registry_action(&self.action) != (*sanction == self.operation))
                {
                    return Err(refused());
                }
            }
            _ if !action_matches(&self.action, REFUND) => return Err(refused()),
            _ => {}
        }
        subject::bounded(self, 2048)?;
        Ok(())
    }
    pub fn matches_consent(&self, c: &PaidResaleConsentV1) -> Result<(), ParticipantError> {
        self.validate()?;
        c.validate()?;
        if c.mode != PaidResaleModeV1::Pending
            || self.operation == c.operation
            || self.operation == c.terms.offer
            || self.operation == c.terms.basis.birth.operation
            || self.operation == c.terms.basis.holder.operation
            || self.operation == c.terms.basis.term.operation
            || self.posted_at_utc < c.accepted_at_utc
        {
            return Err(refused());
        }
        let same = self.at_ic.clock == c.accepted_at_ic.clock;
        match &self.reason {
            PaidResaleDecisionKindV1::ClockChanged {} => {
                if same {
                    return Err(refused());
                }
            }
            PaidResaleDecisionKindV1::RegistryRevoked {
                licence,
                generation,
                term_digest,
                ..
            } => {
                let b = &c.terms.basis;
                if *licence != b.licence
                    || *generation != b.term.generation
                    || *term_digest != b.term.digest
                    || (same && self.at_ic.at < c.accepted_at_ic.at)
                    || self.at_ic.at >= b.term.window.grace_end
                {
                    return Err(refused());
                }
                // The separately proved actual sanction captures this exact
                // term. Expired immutable sanctions are never property bans.
            }
            PaidResaleDecisionKindV1::LastClaimClosed { next_transition } => {
                if !same
                    || self.at_ic.at < c.accepted_at_ic.at
                    || *next_transition <= c.claims.next_transition
                {
                    return Err(refused());
                }
            }
            PaidResaleDecisionKindV1::DeadlineReached {
                next_transition,
                open_paid,
            } => {
                if !c.deadline_reached(&self.at_ic)?
                    || *next_transition < c.claims.next_transition
                    || *open_paid > c.claims.open_paid
                {
                    return Err(refused());
                }
            }
            PaidResaleDecisionKindV1::RecipientUnavailable {}
            | PaidResaleDecisionKindV1::MonetaryBindingChanged {} => {
                if !same || self.at_ic.at < c.accepted_at_ic.at {
                    return Err(refused());
                }
            }
        }
        Ok(())
    }
}
