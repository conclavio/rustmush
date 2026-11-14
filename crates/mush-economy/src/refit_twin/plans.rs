//! Fixed money-only native alphabet. The physical images are other owners' (Inventory and
//! Tactics for a bay refit, Core and Tactics for a core refit); this plan never invokes mutable
//! custody callbacks or changes a title.
use super::{Leg, Policy, Subject};
use crate::plan::{EconomyPlan, HoldOp, RequestedLine};
use crate::record::{EconomicCause, HoldPurpose, IdempotencyKey};
use crate::{Amount, HoldId};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
pub(crate) const CORRELATION_BYTES: usize = 16_384;
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound = "")]
pub struct FundingIntent<M: Leg> {
    pub subject: M::Subject,
    pub policy: M::Policy,
    pub supplier: M::Supplier,
}
impl<M: Leg> FundingIntent<M> {
    pub fn validate(&self) -> Result<(), M::Error> {
        M::matches_policy(&self.subject, &self.policy)?;
        M::supplier_validate(&self.supplier, &self.policy, &self.subject)
    }
    /// The fee the leg charges ([`Leg::price`]).
    pub fn price(&self) -> Result<Amount, M::Error> {
        M::price(self)
    }
    /// The account the fee pays ([`Leg::provider_account`]).
    pub fn provider_account(&self) -> Result<crate::AccountId, M::Error> {
        M::provider_account(self)
    }
}
/// One phase of the correlation a plan carries, as the owner reads it. Its wire form is the
/// family's own `Proof` ([`Leg::seal`], [`Leg::open`]); `part` is that record's `refit` or
/// `replacement`.
pub enum Phase<M: Leg> {
    Accept {
        format: u16,
        commit: CommitToken,
        action: DefinitionRef,
        intent: Box<FundingIntent<M>>,
    },
    Install {
        format: u16,
        acceptance: CommitToken,
        commit: CommitToken,
        action: DefinitionRef,
        at: ClockStamp,
        part: DefinitionDigest,
    },
    Cancel {
        format: u16,
        acceptance: CommitToken,
        commit: CommitToken,
        action: DefinitionRef,
        at: ClockStamp,
    },
}
pub(super) fn correlation<M: Leg>(p: &M::Proof) -> Result<String, M::Error> {
    String::from_utf8(M::bytes(p, CORRELATION_BYTES)?).map_err(|_| M::INVALID)
}
pub(crate) fn decode_correlation<M: Leg>(text: &str) -> Result<M::Proof, M::Error> {
    if text.len() > CORRELATION_BYTES {
        return Err(M::CAPACITY);
    }
    let p: M::Proof = serde_json::from_str(text).map_err(|_| M::INVALID)?;
    if correlation::<M>(&p)? != text {
        return Err(M::INTEGRITY);
    }
    Ok(p)
}
fn base<M: Leg>(
    intent: &FundingIntent<M>,
    acceptance: CommitToken,
    suffix: &str,
    proof: &M::Proof,
    lines: Vec<RequestedLine>,
) -> Result<EconomyPlan, M::Error> {
    let mut p = EconomyPlan::new(
        IdempotencyKey::new(M::NAMESPACE, format!("{}/{}", acceptance.to_hex(), suffix)),
        PrincipalId::System(intent.policy.issuer().clone()),
        EconomicCause::Ruleset {
            namespace: M::NAMESPACE.into(),
            correlation: correlation::<M>(proof)?.into(),
        },
        lines,
    );
    p.description = M::PLAN_DESCRIPTION.into();
    Ok(p)
}
pub(crate) fn funding<M: Leg>(
    intent: &FundingIntent<M>,
    commit: CommitToken,
    action: &DefinitionRef,
) -> Result<EconomyPlan, M::Error> {
    intent.validate()?;
    if *action != M::action_ref(&intent.policy, M::ACCEPT_ACTION)?
        || commit == intent.subject.offer()
        || M::commit_reused(&intent.subject, commit)
    {
        return Err(M::INTEGRITY);
    }
    let mut p = base(
        intent,
        commit,
        "accept",
        &M::seal(Phase::Accept {
            format: 1,
            commit,
            action: action.clone(),
            intent: Box::new(intent.clone()),
        }),
        vec![],
    )?;
    p.hold_ops.push(HoldOp::Place {
        account: intent.subject.payer(),
        amount: intent.price()?,
        beneficiary: Some(PrincipalId::System(intent.policy.issuer().clone())),
        purpose: HoldPurpose::Reservation {
            kind: M::HOLD_PURPOSE.into(),
            id: intent.subject.order(),
        },
        expires: None,
    });
    Ok(p)
}
pub(crate) fn terminal<M: Leg>(
    intent: &FundingIntent<M>,
    acceptance: CommitToken,
    hold: HoldId,
    commit: CommitToken,
    action: &DefinitionRef,
    at: &ClockStamp,
    part: Option<DefinitionDigest>,
) -> Result<EconomyPlan, M::Error> {
    intent.validate()?;
    let install = part.is_some();
    if part.is_some_and(|d| d.as_bytes() == &[0; 32]) {
        return Err(M::INTEGRITY);
    }
    let id = if install {
        M::INSTALL_ACTION
    } else {
        M::CANCEL_ACTION
    };
    if *action != M::action_ref(&intent.policy, id)?
        || [acceptance, intent.subject.offer()].contains(&commit)
        || M::commit_reused(&intent.subject, commit)
        || !matches!(at.clock,ClockRef::WorldIc{generation}if generation>0)
    {
        return Err(M::INTEGRITY);
    }
    let mut lines = vec![];
    if install {
        if at.clock != intent.subject.accepted_at().clock
            || at.at < intent.subject.accepted_at().at
            || at.at >= intent.subject.expires_at().at
        {
            return Err(M::INTEGRITY);
        }
        lines.push(RequestedLine {
            account: intent.subject.payer(),
            delta: intent.price()?.checked_neg().ok_or(M::INVALID)?,
            memo: M::LINE_MEMO.into(),
        });
        lines.push(RequestedLine {
            account: intent.provider_account()?,
            delta: intent.price()?,
            memo: M::LINE_MEMO.into(),
        });
    }
    let proof = if install {
        Phase::Install {
            format: 1,
            acceptance,
            commit,
            action: action.clone(),
            at: at.clone(),
            part: part.ok_or(M::INTEGRITY)?,
        }
    } else {
        Phase::Cancel {
            format: 1,
            acceptance,
            commit,
            action: action.clone(),
            at: at.clone(),
        }
    };
    let mut p = base(
        intent,
        acceptance,
        if install { "install" } else { "cancel" },
        &M::seal(proof),
        lines,
    )?;
    p.hold_ops.push(if install {
        HoldOp::Capture {
            hold,
            amount: intent.price()?,
        }
    } else {
        HoldOp::Release { hold }
    });
    Ok(p)
}
