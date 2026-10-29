//! Fixed money-only native alphabet. Core and Tactics own the physical images;
//! this plan never invokes mutable custody callbacks or changes a title.
use super::{
    HOLD_PURPOSE, NAMESPACE, RefitError as Error, RefitOfferAttribution, RefitPaymentPolicy,
    RefitSubject,
};
use crate::plan::{EconomyPlan, HoldOp, RequestedLine};
use crate::record::{EconomicCause, HoldPurpose, IdempotencyKey};
use crate::{Amount, HoldId};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
pub(super) const CORRELATION_BYTES: usize = 16_384;
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FundingIntent {
    pub subject: RefitSubject,
    pub policy: RefitPaymentPolicy,
    pub supplier: RefitOfferAttribution,
}
impl FundingIntent {
    pub fn validate(&self) -> Result<(), Error> {
        self.subject.matches_policy(&self.policy)?;
        self.supplier.validate(&self.policy, &self.subject)
    }
    /// The subject's recipe's fee (doc 17h cut M8b C2: `core_ungovern`
    /// charges `ungovern_minor`); a recipe the policy does not compose is refused.
    pub fn price(&self) -> Result<Amount, Error> {
        self.policy.price_for(self.subject.kind)
    }
    /// The account the subject's recipe pays; refused as `price`.
    pub fn provider_account(&self) -> Result<crate::AccountId, Error> {
        Ok(self.policy.site(self.subject.kind)?.provider.account)
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Proof {
    Accept {
        format: u16,
        commit: CommitToken,
        action: DefinitionRef,
        intent: Box<FundingIntent>,
    },
    Install {
        format: u16,
        acceptance: CommitToken,
        commit: CommitToken,
        action: DefinitionRef,
        at: ClockStamp,
        replacement: DefinitionDigest,
    },
    Cancel {
        format: u16,
        acceptance: CommitToken,
        commit: CommitToken,
        action: DefinitionRef,
        at: ClockStamp,
    },
}
pub(super) fn correlation(p: &Proof) -> Result<String, Error> {
    String::from_utf8(super::subject::bytes(p, CORRELATION_BYTES)?).map_err(|_| Error::Invalid)
}
pub(super) fn decode_correlation(text: &str) -> Result<Proof, Error> {
    if text.len() > CORRELATION_BYTES {
        return Err(Error::Capacity);
    }
    let p: Proof = serde_json::from_str(text).map_err(|_| Error::Invalid)?;
    if correlation(&p)? != text {
        return Err(Error::Integrity);
    }
    Ok(p)
}
fn base(
    intent: &FundingIntent,
    acceptance: CommitToken,
    suffix: &str,
    proof: &Proof,
    lines: Vec<RequestedLine>,
) -> Result<EconomyPlan, Error> {
    let mut p = EconomyPlan::new(
        IdempotencyKey::new(NAMESPACE, format!("{}/{}", acceptance.to_hex(), suffix)),
        PrincipalId::System(intent.policy.issuer.clone()),
        EconomicCause::Ruleset {
            namespace: NAMESPACE.into(),
            correlation: correlation(proof)?.into(),
        },
        lines,
    );
    p.description = "Certified core fitting".into();
    Ok(p)
}
pub(super) fn funding(
    intent: &FundingIntent,
    commit: CommitToken,
    action: &DefinitionRef,
) -> Result<EconomyPlan, Error> {
    intent.validate()?;
    if *action != intent.policy.action_ref(super::ACCEPT_ACTION)?
        || commit == intent.subject.offer
        || commit == intent.subject.core_acceptance
    {
        return Err(Error::Integrity);
    }
    let mut p = base(
        intent,
        commit,
        "accept",
        &Proof::Accept {
            format: 1,
            commit,
            action: action.clone(),
            intent: Box::new(intent.clone()),
        },
        vec![],
    )?;
    p.hold_ops.push(HoldOp::Place {
        account: intent.subject.payer,
        amount: intent.price()?,
        beneficiary: Some(PrincipalId::System(intent.policy.issuer.clone())),
        purpose: HoldPurpose::Reservation {
            kind: HOLD_PURPOSE.into(),
            id: intent.subject.order,
        },
        expires: None,
    });
    Ok(p)
}
pub(super) fn terminal(
    intent: &FundingIntent,
    acceptance: CommitToken,
    hold: HoldId,
    commit: CommitToken,
    action: &DefinitionRef,
    at: &ClockStamp,
    replacement: Option<DefinitionDigest>,
) -> Result<EconomyPlan, Error> {
    intent.validate()?;
    let install = replacement.is_some();
    if replacement.is_some_and(|d| d.as_bytes() == &[0; 32]) {
        return Err(Error::Integrity);
    }
    let id = if install {
        super::INSTALL_ACTION
    } else {
        super::CANCEL_ACTION
    };
    if *action != intent.policy.action_ref(id)?
        || [
            acceptance,
            intent.subject.offer,
            intent.subject.core_acceptance,
        ]
        .contains(&commit)
        || !matches!(at.clock,ClockRef::WorldIc{generation}if generation>0)
    {
        return Err(Error::Integrity);
    }
    let mut lines = vec![];
    if install {
        if at.clock != intent.subject.accepted_at.clock
            || at.at < intent.subject.accepted_at.at
            || at.at >= intent.subject.expires_at.at
        {
            return Err(Error::Integrity);
        }
        lines.push(RequestedLine {
            account: intent.subject.payer,
            delta: intent.price()?.checked_neg().ok_or(Error::Invalid)?,
            memo: "core fitting".into(),
        });
        lines.push(RequestedLine {
            account: intent.provider_account()?,
            delta: intent.price()?,
            memo: "core fitting".into(),
        });
    }
    let proof = if install {
        Proof::Install {
            format: 1,
            acceptance,
            commit,
            action: action.clone(),
            at: at.clone(),
            replacement: replacement.ok_or(Error::Integrity)?,
        }
    } else {
        Proof::Cancel {
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
        &proof,
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
