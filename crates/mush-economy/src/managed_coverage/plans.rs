//! Exact inert plans. Only registered native preparation constructs the private
//! proof accepted by the Economy engine. Deserialized intent grants no rights.
use super::*;
use crate::plan::{AssetOp, AssetRevision, EconomyPlan, HoldOp, HoldRevision, RequestedLine};
use crate::record::{
    EconomicAsset, EconomicCause, EconomicVisibility, Encumbrance, HoldPurpose, IdempotencyKey,
};
use crate::{ContractId, HoldId};
use mush_core::CommitToken;
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BindingIntent {
    pub consent: CoverageConsentV1,
    pub contract: ContractId,
    pub hold: HoldId,
    pub title: EconomicAsset,
}
impl BindingIntent {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        self.consent.validate()?;
        let c = &self.consent;
        if self.title.object != c.basis.hull
            || self.title.title_owner != PrincipalId::Object(c.terms.buyer)
            || self.title.scope.is_some()
            || self.title.revision != c.basis.hull_title_revision
            || self.title.revision.checked_add(3).is_none()
            || self.title.lifecycle != crate::record::AssetLifecycle::Active
            || self.title.frozen
            || self.title.frozen_reason.is_some()
            || self.title.stolen.is_some()
            || !self.title.encumbrances.is_empty()
        {
            return Err(refused());
        }
        super::contract::active(c, self.contract, self.hold)?;
        bounded_title(&self.title)?;
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Proof {
    Bind {
        format: u16,
        intent: BindingIntent,
    },
    Terminal {
        format: u16,
        binding: CommitToken,
        decision: CoverageDecisionV1,
        title: EconomicAsset,
    },
}
pub(super) fn encode(proof: &Proof) -> Result<String, ParticipantError> {
    let bytes = mush_core::canonical_json_bytes_with_limit(
        &serde_json::to_value(proof).map_err(|_| refused())?,
        MAX_CORRELATION_BYTES,
    )
    .map_err(|_| refused())?;
    String::from_utf8(bytes).map_err(|_| refused())
}
pub(super) fn decode(raw: &str) -> Result<Proof, ParticipantError> {
    if raw.len() > MAX_CORRELATION_BYTES {
        return Err(refused());
    }
    let proof: Proof = serde_json::from_str(raw).map_err(|_| refused())?;
    if encode(&proof)? != raw {
        return Err(refused());
    }
    match &proof {
        Proof::Bind { format: 1, intent } => intent.validate()?,
        Proof::Terminal {
            format: 1,
            binding,
            decision,
            ..
        } => {
            decision.validate()?;
            if *binding == decision.operation {
                return Err(refused());
            }
        }
        _ => return Err(refused()),
    }
    Ok(proof)
}
pub(super) fn bind(intent: &BindingIntent) -> Result<EconomyPlan, ParticipantError> {
    intent.validate()?;
    let c = &intent.consent;
    let t = &c.terms;
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new(NAMESPACE, format!("{}/bind", c.operation.to_hex())),
        PrincipalId::Object(t.buyer),
        EconomicCause::Ruleset {
            namespace: NAMESPACE.into(),
            correlation: encode(&Proof::Bind {
                format: 1,
                intent: intent.clone(),
            })?
            .into(),
        },
        vec![
            RequestedLine {
                account: t.payer,
                delta: t.premium.checked_neg().ok_or_else(refused)?,
                memo: "coverage premium".into(),
            },
            RequestedLine {
                account: t.treasury,
                delta: t.premium,
                memo: "coverage premium".into(),
            },
        ],
    );
    plan.description = "Bind hull destruction coverage".into();
    plan.visibility = EconomicVisibility::Parties;
    plan.hold_ops.push(HoldOp::Place {
        account: t.treasury,
        amount: t.liability,
        beneficiary: Some(crate::api::contract_principal()),
        purpose: HoldPurpose::Escrow {
            contract: intent.contract,
        },
        expires: None,
    });
    plan.asset_ops.push(AssetOp::Encumber {
        object: c.basis.hull,
        encumbrance: Encumbrance::Collateral {
            contract: intent.contract,
        },
    });
    plan.expected_asset_revisions.push(AssetRevision {
        object: c.basis.hull,
        revision: c.basis.hull_title_revision,
    });
    Ok(plan)
}
pub(super) fn terminal(
    intent: &BindingIntent,
    decision: &CoverageDecisionV1,
    title: &EconomicAsset,
) -> Result<EconomyPlan, ParticipantError> {
    intent.validate()?;
    decision.matches_consent(&intent.consent)?;
    let c = &intent.consent;
    let t = &c.terms;
    let pays = decision.reason.pays();
    if title.object != c.basis.hull
        || title.title_owner != PrincipalId::Object(t.buyer)
        || title.scope.is_some()
        || title.lifecycle != crate::record::AssetLifecycle::Active
        || title.frozen
        || title.frozen_reason.is_some()
        || title.stolen.is_some()
        || title.encumbrances
            != vec![Encumbrance::Collateral {
                contract: intent.contract,
            }]
        || title.revision <= c.basis.hull_title_revision
        || title.revision.checked_add(2).is_none()
    {
        return Err(refused());
    }
    bounded_title(title)?;
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new(NAMESPACE, format!("{}/terminal", c.operation.to_hex())),
        PrincipalId::Object(decision.actor),
        EconomicCause::Ruleset {
            namespace: NAMESPACE.into(),
            correlation: encode(&Proof::Terminal {
                format: 1,
                binding: c.operation,
                decision: decision.clone(),
                title: title.clone(),
            })?
            .into(),
        },
        if pays {
            vec![
                RequestedLine {
                    account: t.treasury,
                    delta: t.liability.checked_neg().ok_or_else(refused)?,
                    memo: "coverage claim".into(),
                },
                RequestedLine {
                    account: t.payer,
                    delta: t.liability,
                    memo: "coverage claim".into(),
                },
            ]
        } else {
            Vec::new()
        },
    );
    plan.description = if pays {
        "Pay hull destruction claim"
    } else {
        "Release hull coverage liability"
    }
    .into();
    plan.visibility = EconomicVisibility::Parties;
    plan.expected_hold_revisions.push(HoldRevision {
        hold: intent.hold,
        revision: 1,
    });
    plan.hold_ops.push(if pays {
        HoldOp::Capture {
            hold: intent.hold,
            amount: t.liability,
        }
    } else {
        HoldOp::Release { hold: intent.hold }
    });
    plan.expected_asset_revisions.push(AssetRevision {
        object: c.basis.hull,
        revision: title.revision,
    });
    plan.asset_ops.push(AssetOp::Release {
        object: c.basis.hull,
        encumbrance: Encumbrance::Collateral {
            contract: intent.contract,
        },
    });
    if pays {
        plan.asset_ops.push(AssetOp::TransferTitle {
            object: c.basis.hull,
            to: PrincipalId::Organization(t.insurer),
            to_scope: Some(t.scope),
        });
    }
    Ok(plan)
}

fn bounded_title(title: &EconomicAsset) -> Result<(), ParticipantError> {
    mush_core::canonical_json_bytes_with_limit(
        &serde_json::to_value(title).map_err(|_| refused())?,
        MAX_TITLE_BYTES,
    )
    .map_err(|_| refused())?;
    Ok(())
}
