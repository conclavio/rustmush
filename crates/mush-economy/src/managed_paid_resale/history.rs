//! Actual journal-derived monetary history. Serialized values are closed data;
//! only the separately completed native inventory audit supplies a live view.
use super::plans::Intent;
use super::*;
use crate::contract::{ContractState, EconomicContract};
use crate::record::{
    EconomicTransaction, Hold, HoldEffectKind, HoldPurpose, HoldStatus, IdempotencyKey,
};
use crate::{ContractId, HoldId, TransactionId};
use mush_core::DefinitionDigest;
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};

fn required<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaidResaleOutcome {
    Funded,
    Settled,
    Refunded,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaidResaleAcceptanceReceiptV1 {
    transaction: TransactionId,
    journal_seq: u64,
    plan_hash: [u8; 32],
    #[serde(deserialize_with = "required")]
    hold: Option<Hold>,
}
impl PaidResaleAcceptanceReceiptV1 {
    pub const fn transaction(&self) -> TransactionId {
        self.transaction
    }
    pub const fn journal_seq(&self) -> u64 {
        self.journal_seq
    }
    pub(super) fn plan_hash(&self) -> &[u8; 32] {
        &self.plan_hash
    }
    pub fn hold(&self) -> Option<HoldId> {
        self.hold.as_ref().map(|h| h.id)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaidResaleTerminalReceiptV1 {
    decision: PaidResaleDecisionV1,
    transaction: TransactionId,
    journal_seq: u64,
    plan_hash: [u8; 32],
    hold: Hold,
}
impl PaidResaleTerminalReceiptV1 {
    pub(super) fn plan_hash(&self) -> &[u8; 32] {
        &self.plan_hash
    }
    pub fn decision(&self) -> &PaidResaleDecisionV1 {
        &self.decision
    }
    pub const fn transaction(&self) -> TransactionId {
        self.transaction
    }
    pub const fn journal_seq(&self) -> u64 {
        self.journal_seq
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaidResaleHistoryV1 {
    intent: Intent,
    acceptance: Box<PaidResaleAcceptanceReceiptV1>,
    #[serde(deserialize_with = "required")]
    terminal: Option<Box<PaidResaleTerminalReceiptV1>>,
}
impl PaidResaleHistoryV1 {
    pub fn consent(&self) -> &PaidResaleConsentV1 {
        &self.intent.consent
    }
    pub const fn contract_id(&self) -> ContractId {
        self.intent.contract
    }
    pub fn acceptance(&self) -> &PaidResaleAcceptanceReceiptV1 {
        &self.acceptance
    }
    pub fn terminal(&self) -> Option<&PaidResaleTerminalReceiptV1> {
        self.terminal.as_deref()
    }
    pub(super) fn intent(&self) -> &Intent {
        &self.intent
    }
    pub(super) fn hold(&self) -> Option<&Hold> {
        self.terminal
            .as_ref()
            .map(|t| &t.hold)
            .or(self.acceptance.hold.as_ref())
    }
    pub fn outcome(&self) -> PaidResaleOutcome {
        if self.consent().mode == PaidResaleModeV1::Immediate {
            PaidResaleOutcome::Settled
        } else {
            match &self.terminal {
                None => PaidResaleOutcome::Funded,
                Some(t) if t.decision.reason.settles() => PaidResaleOutcome::Settled,
                Some(_) => PaidResaleOutcome::Refunded,
            }
        }
    }
    pub fn digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value_with_limit(
            "economy.paid_resale.history.v1",
            &serde_json::to_value(self).map_err(|_| refused())?,
            MAX_HISTORY_BYTES,
        )
        .map_err(|_| refused())
    }
    pub fn decode(raw: &[u8]) -> Result<Self, ParticipantError> {
        if raw.is_empty() || raw.len() > MAX_HISTORY_BYTES {
            return Err(refused());
        }
        let r: Self = serde_json::from_slice(raw).map_err(|_| refused())?;
        r.validate()?;
        if subject::bounded(&r, MAX_HISTORY_BYTES)? != raw {
            return Err(refused());
        }
        Ok(r)
    }
    pub(super) fn contract(&self) -> Result<EconomicContract, ParticipantError> {
        let mut c = super::contract::accepted(&self.intent, self.acceptance.transaction)?;
        if let Some(t) = &self.terminal {
            c.state = if t.decision.reason.settles() {
                ContractState::Settled {
                    transaction: t.transaction,
                }
            } else {
                ContractState::Cancelled
            };
            c.revision = 2;
        }
        subject::bounded(&c, MAX_CONTRACT_BYTES)?;
        Ok(c)
    }
    pub fn verify_documents(
        &self,
        contract: &EconomicContract,
        hold: Option<&Hold>,
    ) -> Result<(), ParticipantError> {
        self.validate()?;
        if contract != &self.contract()? || hold != self.hold() {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn validate(&self) -> Result<(), ParticipantError> {
        self.intent.validate()?;
        let c = self.consent();
        let a = &self.acceptance;
        if a.plan_hash != crate::hash::plan_hash(&plans::acceptance(&self.intent)?) {
            return Err(refused());
        }
        match c.mode {
            PaidResaleModeV1::Immediate => {
                if a.hold.is_some() || self.terminal.is_some() {
                    return Err(refused());
                }
            }
            PaidResaleModeV1::Pending => {
                let h = a.hold.as_ref().ok_or_else(refused)?;
                if Some(h.id) != self.intent.hold
                    || h.account != c.payer
                    || h.amount != c.terms.price
                    || h.status != HoldStatus::Held
                    || h.placed_by != PrincipalId::Object(c.terms.buyer)
                    || h.beneficiary != Some(crate::api::contract_principal())
                    || h.purpose
                        != (HoldPurpose::Escrow {
                            contract: self.intent.contract,
                        })
                    || h.holder
                        != IdempotencyKey::new(NAMESPACE, format!("{}/fund", c.operation.to_hex()))
                    || h.expires.is_some()
                    || h.revision != 1
                    || h.last_journal_seq != a.journal_seq
                {
                    return Err(refused());
                }
                if let Some(t) = &self.terminal {
                    t.decision.matches_consent(c)?;
                    let mut expected = h.clone();
                    expected.status = if t.decision.reason.settles() {
                        HoldStatus::Consumed {
                            transaction: t.transaction,
                            amount: c.terms.price,
                        }
                    } else {
                        HoldStatus::Released
                    };
                    expected.revision = 2;
                    expected.last_journal_seq = t.journal_seq;
                    if t.transaction == a.transaction
                        || t.journal_seq <= a.journal_seq
                        || t.hold != expected
                        || t.plan_hash
                            != crate::hash::plan_hash(&plans::terminal(&self.intent, &t.decision)?)
                    {
                        return Err(refused());
                    }
                }
            }
        }
        subject::bounded(self, MAX_HISTORY_BYTES)?;
        self.contract()?;
        Ok(())
    }
    pub(super) fn require_terminal_room(&self) -> Result<(), ParticipantError> {
        self.validate()?;
        if self.consent().mode == PaidResaleModeV1::Immediate {
            return Ok(());
        }
        if self.terminal.is_some() {
            return Err(refused());
        }
        let mut hold = self.acceptance.hold.clone().ok_or_else(refused)?;
        hold.status = HoldStatus::Consumed {
            transaction: TransactionId(u64::MAX),
            amount: self.consent().terms.price,
        };
        hold.revision = 2;
        hold.last_journal_seq = u64::MAX;
        let shell = serde_json::json!({"decision":null,"transaction":u64::MAX,"journal_seq":u64::MAX,"plan_hash":vec![255u8;32],"hold":hold});
        let maximum = subject::bounded(&shell, MAX_HISTORY_BYTES)?
            .len()
            .checked_sub(4)
            .and_then(|n| n.checked_add(2048))
            .ok_or_else(refused)?;
        if subject::bounded(self, MAX_HISTORY_BYTES)?
            .len()
            .checked_sub(4)
            .and_then(|n| n.checked_add(maximum))
            .is_none_or(|n| n > MAX_HISTORY_BYTES)
        {
            return Err(refused());
        }
        let mut c = self.contract()?;
        c.state = ContractState::Settled {
            transaction: TransactionId(u64::MAX),
        };
        c.revision = 2;
        subject::bounded(&c, MAX_CONTRACT_BYTES)?;
        let write = crate::registry_filing::json_write(crate::api::contract_key(c.id), &c)?;
        subject::bounded(&write, MAX_CONTRACT_BYTES)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "history_capacity_tests.rs"]
mod capacity_tests;
fn transaction_matches(
    tx: &EconomicTransaction,
    plan: &crate::EconomyPlan,
    i: &Intent,
) -> Result<(), ParticipantError> {
    if tx.idempotency != plan.idempotency
        || tx.actor != plan.actor
        || tx.cause != plan.cause
        || tx.description != plan.description
        || tx.visibility != plan.visibility
        || tx.plan_hash != crate::hash::plan_hash(plan)
        || tx.reverses.is_some()
        || tx.consumed_approval.is_some()
        || !tx.custody_effects.is_empty()
        || !tx.asset_effects.is_empty()
    {
        return Err(refused());
    }
    let mut actual: Vec<_> = tx
        .lines
        .iter()
        .map(|l| (l.account, l.delta.minor(), l.memo.as_str()))
        .collect();
    let mut expected: Vec<_> = plan
        .requested_lines
        .iter()
        .map(|l| (l.account, l.delta.minor(), l.memo.as_str()))
        .collect();
    actual.sort();
    expected.sort();
    if actual != expected
        || tx
            .lines
            .iter()
            .any(|l| l.instrument != i.consent.terms.instrument)
    {
        return Err(refused());
    }
    Ok(())
}
pub(super) fn from_acceptance(
    tx: &EconomicTransaction,
    i: Intent,
) -> Result<PaidResaleHistoryV1, ParticipantError> {
    i.validate()?;
    transaction_matches(tx, &plans::acceptance(&i)?, &i)?;
    if tx.posted_at != i.consent.accepted_at_utc {
        return Err(refused());
    }
    let hold = match i.consent.mode {
        PaidResaleModeV1::Immediate => {
            if !tx.hold_effects.is_empty() {
                return Err(refused());
            }
            None
        }
        PaidResaleModeV1::Pending => {
            let [h] = tx.hold_effects.as_slice() else {
                return Err(refused());
            };
            if h.kind != HoldEffectKind::Placed {
                return Err(refused());
            }
            Some(h.hold.clone())
        }
    };
    let row = PaidResaleHistoryV1 {
        intent: i,
        acceptance: Box::new(PaidResaleAcceptanceReceiptV1 {
            transaction: tx.id,
            journal_seq: tx.journal_seq,
            plan_hash: tx.plan_hash,
            hold,
        }),
        terminal: None,
    };
    row.validate()?;
    row.require_terminal_room()?;
    Ok(row)
}
pub(super) fn with_terminal(
    before: &PaidResaleHistoryV1,
    tx: &EconomicTransaction,
    d: PaidResaleDecisionV1,
) -> Result<PaidResaleHistoryV1, ParticipantError> {
    before.validate()?;
    if before.outcome() != PaidResaleOutcome::Funded || tx.posted_at != d.posted_at_utc {
        return Err(refused());
    }
    transaction_matches(tx, &plans::terminal(&before.intent, &d)?, &before.intent)?;
    let [h] = tx.hold_effects.as_slice() else {
        return Err(refused());
    };
    if h.kind
        != if d.reason.settles() {
            HoldEffectKind::Captured
        } else {
            HoldEffectKind::Released
        }
    {
        return Err(refused());
    }
    let mut row = before.clone();
    row.terminal = Some(Box::new(PaidResaleTerminalReceiptV1 {
        decision: d,
        transaction: tx.id,
        journal_seq: tx.journal_seq,
        plan_hash: tx.plan_hash,
        hold: h.hold.clone(),
    }));
    row.validate()?;
    Ok(row)
}
