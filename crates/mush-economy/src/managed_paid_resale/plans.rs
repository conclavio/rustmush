//! Deterministic private monetary plans. Only the actual configured owner
//! prepares these; serialized intent is not a plan-submission capability.
use super::*;
use crate::plan::{EconomyPlan, HoldOp, HoldRevision, RequestedLine};
use crate::record::{EconomicCause, EconomicVisibility, HoldPurpose, IdempotencyKey};
use crate::{ContractId, HoldId};
use mush_core::CommitToken;
use mush_org::{PrincipalId, SystemPrincipalId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Intent {
    pub consent: PaidResaleConsentV1,
    pub issuer: SystemPrincipalId,
    pub contract: ContractId,
    #[serde(deserialize_with = "required")]
    pub hold: Option<HoldId>,
}
fn required<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
impl Intent {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        self.consent.validate()?;
        if (self.consent.mode == PaidResaleModeV1::Pending) != self.hold.is_some() {
            return Err(refused());
        }
        let proof = match self.consent.mode {
            PaidResaleModeV1::Immediate => Proof::Accept {
                format: 1,
                intent: self.clone(),
            },
            PaidResaleModeV1::Pending => Proof::Fund {
                format: 1,
                intent: self.clone(),
            },
        };
        encode(&proof)?;
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Proof {
    Accept {
        format: u16,
        intent: Intent,
    },
    Fund {
        format: u16,
        intent: Intent,
    },
    Terminal {
        format: u16,
        acceptance: CommitToken,
        decision: PaidResaleDecisionV1,
    },
}
pub(super) fn encode(proof: &Proof) -> Result<String, ParticipantError> {
    String::from_utf8(subject::bounded(proof, MAX_CORRELATION_BYTES)?).map_err(|_| refused())
}
pub(super) fn decode(raw: &str) -> Result<Proof, ParticipantError> {
    if raw.is_empty() || raw.len() > MAX_CORRELATION_BYTES {
        return Err(refused());
    }
    let p: Proof = serde_json::from_str(raw).map_err(|_| refused())?;
    if encode(&p)? != raw {
        return Err(refused());
    }
    match &p {
        Proof::Accept { format: 1, intent }
            if intent.consent.mode == PaidResaleModeV1::Immediate =>
        {
            intent.validate()?
        }
        Proof::Fund { format: 1, intent } if intent.consent.mode == PaidResaleModeV1::Pending => {
            intent.validate()?
        }
        Proof::Terminal {
            format: 1,
            acceptance,
            decision,
        } if *acceptance != decision.operation => decision.validate()?,
        _ => return Err(refused()),
    }
    Ok(p)
}
fn lines(c: &PaidResaleConsentV1) -> Result<Vec<RequestedLine>, ParticipantError> {
    let mut lines = Vec::with_capacity(7);
    // Keep full gross debit and royalty credit distinct, including a buyer who
    // is an ancestor recipient. Native checked balance grouping happens later.
    lines.push(RequestedLine {
        account: c.payer,
        delta: c.terms.price.checked_neg().ok_or_else(refused)?,
        memo: "paid licence resale gross".into(),
    });
    for p in c.terms.payouts(c.payer)? {
        lines.push(RequestedLine {
            account: p.account,
            delta: p.amount,
            memo: "paid licence resale share".into(),
        });
    }
    Ok(lines)
}
pub(super) fn acceptance(i: &Intent) -> Result<EconomyPlan, ParticipantError> {
    i.validate()?;
    let c = &i.consent;
    let immediate = c.mode == PaidResaleModeV1::Immediate;
    let proof = if immediate {
        Proof::Accept {
            format: 1,
            intent: i.clone(),
        }
    } else {
        Proof::Fund {
            format: 1,
            intent: i.clone(),
        }
    };
    let mut p = EconomyPlan::new(
        IdempotencyKey::new(
            NAMESPACE,
            format!(
                "{}/{}",
                c.operation.to_hex(),
                if immediate { "accept" } else { "fund" }
            ),
        ),
        PrincipalId::Object(c.terms.buyer),
        EconomicCause::Ruleset {
            namespace: NAMESPACE.into(),
            correlation: encode(&proof)?.into(),
        },
        if immediate { lines(c)? } else { Vec::new() },
    );
    p.description = if immediate {
        "Paid licence resale payment"
    } else {
        "Paid licence resale funding"
    }
    .into();
    p.visibility = EconomicVisibility::Parties;
    if !immediate {
        p.hold_ops.push(HoldOp::Place {
            account: c.payer,
            amount: c.terms.price,
            beneficiary: Some(crate::api::contract_principal()),
            purpose: HoldPurpose::Escrow {
                contract: i.contract,
            },
            expires: None,
        });
    }
    Ok(p)
}
pub(super) fn terminal(
    i: &Intent,
    d: &PaidResaleDecisionV1,
) -> Result<EconomyPlan, ParticipantError> {
    i.validate()?;
    d.matches_consent(&i.consent)?;
    let hold = i.hold.ok_or_else(refused)?;
    let settle = d.reason.settles();
    let mut p = EconomyPlan::new(
        IdempotencyKey::new(
            NAMESPACE,
            format!(
                "{}/{}",
                i.consent.operation.to_hex(),
                if settle { "settle" } else { "refund" }
            ),
        ),
        PrincipalId::System(i.issuer.clone()),
        EconomicCause::Ruleset {
            namespace: NAMESPACE.into(),
            correlation: encode(&Proof::Terminal {
                format: 1,
                acceptance: i.consent.operation,
                decision: d.clone(),
            })?
            .into(),
        },
        if settle {
            lines(&i.consent)?
        } else {
            Vec::new()
        },
    );
    p.description = if settle {
        "Paid licence resale settlement"
    } else {
        "Paid licence resale refund"
    }
    .into();
    p.visibility = EconomicVisibility::Parties;
    p.expected_hold_revisions
        .push(HoldRevision { hold, revision: 1 });
    p.hold_ops.push(if settle {
        HoldOp::Capture {
            hold,
            amount: i.consent.terms.price,
        }
    } else {
        HoldOp::Release { hold }
    });
    // A paid instrument has no EconomicAsset/title. The paired Roll owner alone
    // changes the exact licence holder in this same acknowledged union.
    Ok(p)
}
