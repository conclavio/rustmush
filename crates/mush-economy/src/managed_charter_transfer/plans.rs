//! Closed deterministic native plans and bounded journal correlation. Structural
//! data is not authorization; only the registered paired owners may prepare it.
use super::{CharterPendingConsentV1, CharterPendingDecisionV1, NAMESPACE, refused};
use crate::plan::{AssetOp, AssetRevision, EconomyPlan, HoldOp, HoldRevision, RequestedLine};
use crate::record::{CatalogRef, EconomicCause, EconomicVisibility, HoldPurpose, IdempotencyKey};
use crate::{ContractId, HoldId};
use mush_core::CommitToken;
use mush_core::participant::ParticipantError;
use mush_org::{PrincipalId, SystemPrincipalId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FundingIntent {
    pub consent: CharterPendingConsentV1,
    pub issuer: SystemPrincipalId,
    pub catalog: CatalogRef,
    pub contract: ContractId,
    pub hold: HoldId,
}
impl FundingIntent {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        self.consent.validate()?;
        super::contract::funded(&self.consent, self.contract, self.hold)?;
        encode(&Proof::Fund {
            format: 1,
            intent: Box::new(self.clone()),
        })?;
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Proof {
    Fund {
        format: u16,
        intent: Box<FundingIntent>,
    },
    Terminal {
        format: u16,
        funding: CommitToken,
        decision: Box<CharterPendingDecisionV1>,
    },
}
pub(super) fn encode(proof: &Proof) -> Result<String, ParticipantError> {
    let bytes = mush_core::canonical_json_bytes_with_limit(
        &serde_json::to_value(proof).map_err(|_| refused())?,
        super::MAX_PENDING_CORRELATION_BYTES,
    )
    .map_err(|_| refused())?;
    String::from_utf8(bytes).map_err(|_| refused())
}
pub(super) fn decode(raw: &str) -> Result<Proof, ParticipantError> {
    if raw.len() > super::MAX_PENDING_CORRELATION_BYTES {
        return Err(refused());
    }
    let proof: Proof = serde_json::from_str(raw).map_err(|_| refused())?;
    if encode(&proof)? != raw {
        return Err(refused());
    }
    match &proof {
        Proof::Fund { format: 1, intent } => intent.consent.validate()?,
        Proof::Terminal {
            format: 1,
            funding,
            decision,
        } => {
            decision.validate()?;
            if *funding == decision.operation {
                return Err(refused());
            }
        }
        _ => return Err(refused()),
    }
    Ok(proof)
}
pub(super) fn funding(intent: &FundingIntent) -> Result<EconomyPlan, ParticipantError> {
    intent.validate()?;
    let c = &intent.consent;
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new(NAMESPACE, format!("{}/fund", c.operation.to_hex())),
        PrincipalId::Object(c.terms.buyer),
        EconomicCause::Ruleset {
            namespace: NAMESPACE.into(),
            correlation: encode(&Proof::Fund {
                format: 1,
                intent: Box::new(intent.clone()),
            })?
            .into(),
        },
        Vec::new(),
    );
    plan.description = "Pending mind charter funding".into();
    plan.visibility = EconomicVisibility::Parties;
    plan.hold_ops.push(HoldOp::Place {
        account: c.payer,
        amount: c.terms.price,
        beneficiary: Some(crate::api::contract_principal()),
        purpose: HoldPurpose::Escrow {
            contract: intent.contract,
        },
        expires: None,
    });
    Ok(plan)
}
pub(super) fn terminal(
    intent: &FundingIntent,
    decision: &CharterPendingDecisionV1,
) -> Result<EconomyPlan, ParticipantError> {
    intent.validate()?;
    decision.matches_consent(&intent.consent)?;
    let c = &intent.consent;
    let settle = decision.reason.settles();
    let transition = if settle { "settle" } else { "refund" };
    let lines = if settle {
        vec![
            RequestedLine {
                account: c.payer,
                delta: c.terms.price.checked_neg().ok_or_else(refused)?,
                memo: "charter escrow".into(),
            },
            RequestedLine {
                account: c.terms.seller_account,
                delta: c.terms.price,
                memo: "charter payout".into(),
            },
        ]
    } else {
        Vec::new()
    };
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new(NAMESPACE, format!("{}/{transition}", c.operation.to_hex())),
        PrincipalId::System(intent.issuer.clone()),
        EconomicCause::Ruleset {
            namespace: NAMESPACE.into(),
            correlation: encode(&Proof::Terminal {
                format: 1,
                funding: c.operation,
                decision: Box::new(decision.clone()),
            })?
            .into(),
        },
        lines,
    );
    plan.description = if settle {
        "Pending mind charter settlement"
    } else {
        "Pending mind charter refund"
    }
    .into();
    plan.visibility = EconomicVisibility::Parties;
    plan.expected_hold_revisions.push(HoldRevision {
        hold: intent.hold,
        revision: 1,
    });
    plan.hold_ops.push(if settle {
        HoldOp::Capture {
            hold: intent.hold,
            amount: c.terms.price,
        }
    } else {
        HoldOp::Release { hold: intent.hold }
    });
    if settle {
        plan.asset_ops.push(AssetOp::TransferTitle {
            object: c.terms.charter,
            to: PrincipalId::Object(c.terms.buyer),
            to_scope: None,
        });
        plan.expected_asset_revisions.push(AssetRevision {
            object: c.terms.charter,
            revision: c.terms.title_revision,
        });
    }
    Ok(plan)
}
