//! One fixed native E4 alphabet. No caller-supplied plans or amounts.
use super::{
    CoreFundingSubject, CoreIntegrationError as Error, CorePaymentPolicy, HOLD_PURPOSE, NAMESPACE,
    consent::CoreOfferAttribution, subject::DiePayment,
};
use crate::plan::{AssetOp, EconomyPlan, HoldOp, RequestedLine};
use crate::record::{EconomicCause, HoldPurpose, IdempotencyKey};
use crate::{Amount, HoldId};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};

pub(super) const CORRELATION_BYTES: usize = 16_384;
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FundingIntent {
    pub subject: CoreFundingSubject,
    pub policy: CorePaymentPolicy,
    pub supplier: CoreOfferAttribution,
    pub payment: DiePayment,
}
impl FundingIntent {
    pub fn validate(&self) -> Result<(), Error> {
        self.subject.validate()?;
        self.policy.validate()?;
        self.supplier.validate(&self.policy, &self.subject)?;
        if self.subject.policy != self.policy.digest()?
            || self.subject.recipe != self.policy.recipe
            || [self.policy.supplier.account, self.policy.integrator.account]
                .contains(&self.subject.payer)
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
    pub fn price(&self) -> Amount {
        self.payment.price(self.subject.class)
    }
}
/// Exact immutable Core output facts, captured only from the real opaque
/// ReservedCoreOutput at live preparation. Decoding is historical data only.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreOutputBirth {
    pub object: ObjId,
    pub reservation: CommitToken,
    pub reservation_action: DefinitionRef,
    pub object_digest: DefinitionDigest,
}
impl CoreOutputBirth {
    pub(super) fn capture(
        output: &mush_core::core_output::ReservedCoreOutput,
        subject: &CoreFundingSubject,
        policy: &CorePaymentPolicy,
        commit: CommitToken,
        action: &DefinitionRef,
    ) -> Result<Self, Error> {
        let order = output.order();
        if output.commit() != commit
            || output.action() != action
            || action != &policy.action_ref(super::COMPLETE_ACTION)?
            || order.id() != subject.order
            || order.intent() != subject.intent
            || order.requester() != subject.requester
            || order.recipe() != &subject.recipe
            || order.facility() != policy.factory
            || order.container() != policy.output_container
        {
            return Err(Error::Integrity);
        }
        let row = Self {
            object: output.object(),
            reservation: output.reservation(),
            reservation_action: output.reservation_action().clone(),
            object_digest: output.object_digest(),
        };
        row.validate(policy)?;
        Ok(row)
    }
    fn validate(&self, policy: &CorePaymentPolicy) -> Result<(), Error> {
        if self.object.0 < 2
            || [policy.factory, policy.output_container].contains(&self.object)
            || self.object_digest.as_bytes() == &[0; 32]
            || self.reservation_action != policy.action_ref("reserve_core_output")?
        {
            return Err(Error::Integrity);
        }
        Ok(())
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
    Complete {
        format: u16,
        acceptance: CommitToken,
        commit: CommitToken,
        action: DefinitionRef,
        at: ClockStamp,
        output: Box<CoreOutputBirth>,
    },
    Cancel {
        format: u16,
        acceptance: CommitToken,
        commit: CommitToken,
        action: DefinitionRef,
        at: ClockStamp,
    },
}
pub(super) fn correlation(proof: &Proof) -> Result<String, Error> {
    let value = serde_json::to_value(proof).map_err(|_| Error::Invalid)?;
    let bytes = mush_core::mechanics::canonical_json_bytes_with_limit(&value, CORRELATION_BYTES)
        .map_err(|_| Error::Capacity)?;
    String::from_utf8(bytes).map_err(|_| Error::Invalid)
}
pub(super) fn decode_correlation(text: &str) -> Result<Proof, Error> {
    if text.len() > CORRELATION_BYTES {
        return Err(Error::Capacity);
    }
    let proof: Proof = serde_json::from_str(text).map_err(|_| Error::Invalid)?;
    if correlation(&proof)? != text {
        return Err(Error::Integrity);
    }
    Ok(proof)
}
fn base(
    intent: &FundingIntent,
    acceptance: CommitToken,
    suffix: &str,
    proof: &Proof,
    lines: Vec<RequestedLine>,
) -> Result<EconomyPlan, Error> {
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new(NAMESPACE, format!("{}/{}", acceptance.to_hex(), suffix)),
        PrincipalId::System(intent.policy.issuer.clone()),
        EconomicCause::Ruleset {
            namespace: NAMESPACE.into(),
            correlation: correlation(proof)?.into(),
        },
        lines,
    );
    plan.description = "Certified core integration".into();
    Ok(plan)
}
pub(super) fn funding(
    intent: &FundingIntent,
    commit: CommitToken,
    action: &DefinitionRef,
) -> Result<EconomyPlan, Error> {
    intent.validate()?;
    if *action != intent.policy.action_ref(super::ACCEPT_ACTION)? || commit == intent.subject.offer
    {
        return Err(Error::Integrity);
    }
    let proof = Proof::Accept {
        format: 1,
        commit,
        action: action.clone(),
        intent: Box::new(intent.clone()),
    };
    let mut plan = base(intent, commit, "accept", &proof, Vec::new())?;
    // Expiry is enforced against the retained versioned clock by this closed
    // producer. Generic sweep must not age out half of a coupled work order.
    plan.hold_ops.push(HoldOp::Place {
        account: intent.subject.payer,
        amount: intent.price(),
        beneficiary: Some(PrincipalId::System(intent.policy.issuer.clone())),
        purpose: HoldPurpose::Reservation {
            kind: HOLD_PURPOSE.into(),
            id: intent.subject.order,
        },
        expires: None,
    });
    Ok(plan)
}
pub(super) fn terminal(
    intent: &FundingIntent,
    acceptance: CommitToken,
    hold: HoldId,
    commit: CommitToken,
    action: &DefinitionRef,
    at: &ClockStamp,
    output: Option<&CoreOutputBirth>,
) -> Result<EconomyPlan, Error> {
    intent.validate()?;
    let complete = output.is_some();
    let id = if complete {
        super::COMPLETE_ACTION
    } else {
        super::CANCEL_ACTION
    };
    if *action != intent.policy.action_ref(id)?
        || [acceptance, intent.subject.offer].contains(&commit)
        || !matches!(at.clock,mush_core::ClockRef::WorldIc{generation}if generation>0)
    {
        return Err(Error::Integrity);
    }
    let mut lines = Vec::new();
    if complete {
        if at.clock != intent.subject.accepted_at.clock
            || at.at < intent.subject.accepted_at.at
            || at.at >= intent.subject.expires_at.at
        {
            return Err(Error::Integrity);
        }
        lines.push(RequestedLine {
            account: intent.subject.payer,
            delta: intent.price().checked_neg().ok_or(Error::Invalid)?,
            memo: "certified core".into(),
        });
        lines.push(RequestedLine {
            account: intent.policy.integrator.account,
            delta: Amount::from_minor(intent.subject.class.integration_minor()),
            memo: "core integration".into(),
        });
        if intent.payment == DiePayment::SupplierStock {
            lines.push(RequestedLine {
                account: intent.policy.supplier.account,
                delta: Amount::from_minor(intent.subject.class.die_minor()),
                memo: "certified die".into(),
            });
        }
    }
    let proof = if let Some(output) = output {
        output.validate(&intent.policy)?;
        if [acceptance, commit].contains(&output.reservation) {
            return Err(Error::Integrity);
        }
        Proof::Complete {
            format: 1,
            acceptance,
            commit,
            action: action.clone(),
            at: at.clone(),
            output: Box::new(output.clone()),
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
    let mut plan = base(
        intent,
        acceptance,
        if complete { "complete" } else { "cancel" },
        &proof,
        lines,
    )?;
    if let Some(output) = output {
        plan.hold_ops.push(HoldOp::Capture {
            hold,
            amount: intent.price(),
        });
        plan.asset_ops.push(AssetOp::Title {
            object: output.object,
            owner: PrincipalId::Object(intent.subject.requester),
            scope: None,
            catalog: Some(
                intent
                    .policy
                    .catalogs
                    .get(intent.subject.class.catalog_index())
                    .ok_or(Error::Invalid)?
                    .clone(),
            ),
        });
    } else {
        plan.hold_ops.push(HoldOp::Release { hold });
    }
    Ok(plan)
}
