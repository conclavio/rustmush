//! Separate journal authority for an existing renewal offer. No current contract
//! or decoded row can manufacture this module's runtime replay provenance.
use super::{
    FUND_ACTION, GRANT_ACTION, NAMESPACE, REFUND_ACTION, action_matches, contract,
    history::OfferRecord, refused,
};
use crate::contract::{Acceptance, ContractState, EconomicContract};
use crate::plan::{EconomyPlan, HoldOp, RequestedLine};
use crate::record::{
    EconomicCause, EconomicTransaction, EconomicVisibility, Hold, HoldEffectKind, HoldPurpose,
    HoldStatus, IdempotencyKey,
};
use crate::{ContractId, HoldId, TransactionId};
use mush_core::participant::ParticipantError;
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionRef};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(super) const MAX_CORRELATION_BYTES: usize = 8192;
pub(super) const MAX_HISTORY_BYTES: usize = 12288;
pub(super) const TERMINAL_RESERVE_BYTES: usize = 2048;
pub(super) const MAX_HISTORIES: usize = 32768;
pub const MAX_RENEWAL_RECEIPT_BYTES: usize = 1024;
fn versioned(stamp: &ClockStamp) -> bool {
    matches!(stamp.clock, ClockRef::WorldIc { generation } if generation != 0)
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RenewalFundingReceiptV1 {
    format: u16,
    commit: CommitToken,
    action: DefinitionRef,
    contract: ContractId,
    hold: HoldId,
    transaction: TransactionId,
    journal_seq: u64,
    consent_hash: [u8; 32],
    at_ic: ClockStamp,
    posted_at_utc: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FundingWire {
    format: u16,
    commit: CommitToken,
    action: DefinitionRef,
    contract: ContractId,
    hold: HoldId,
    transaction: TransactionId,
    journal_seq: u64,
    consent_hash: [u8; 32],
    at_ic: ClockStamp,
    posted_at_utc: i64,
}
impl<'de> Deserialize<'de> for RenewalFundingReceiptV1 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = FundingWire::deserialize(d)?;
        let value = Self {
            format: w.format,
            commit: w.commit,
            action: w.action,
            contract: w.contract,
            hold: w.hold,
            transaction: w.transaction,
            journal_seq: w.journal_seq,
            consent_hash: w.consent_hash,
            at_ic: w.at_ic,
            posted_at_utc: w.posted_at_utc,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl RenewalFundingReceiptV1 {
    pub fn decode(bytes: &[u8]) -> Result<Self, RenewalAuditError> {
        let row = decode_receipt(bytes)?;
        Self::validate(&row)?;
        Ok(row)
    }
    fn validate(&self) -> Result<(), RenewalAuditError> {
        if self.format != 1
            || !action_matches(&self.action, FUND_ACTION)
            || !versioned(&self.at_ic)
            || serde_json::to_vec(self)
                .map_err(|_| RenewalAuditError::Refused)?
                .len()
                > MAX_RENEWAL_RECEIPT_BYTES
        {
            return Err(RenewalAuditError::Refused);
        }
        Ok(())
    }
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn contract(&self) -> ContractId {
        self.contract
    }
    pub const fn hold(&self) -> HoldId {
        self.hold
    }
    pub const fn transaction(&self) -> TransactionId {
        self.transaction
    }
    pub const fn journal_seq(&self) -> u64 {
        self.journal_seq
    }
    pub const fn consent_hash(&self) -> &[u8; 32] {
        &self.consent_hash
    }
    pub fn at_ic(&self) -> &ClockStamp {
        &self.at_ic
    }
    pub const fn posted_at_utc(&self) -> i64 {
        self.posted_at_utc
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenewalOutcome {
    Granted,
    Refunded,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RenewalTerminalReceiptV1 {
    format: u16,
    commit: CommitToken,
    action: DefinitionRef,
    outcome: RenewalOutcome,
    transaction: TransactionId,
    journal_seq: u64,
    at_ic: ClockStamp,
    posted_at_utc: i64,
    issued_at_ic: Option<ClockStamp>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalWire {
    format: u16,
    commit: CommitToken,
    action: DefinitionRef,
    outcome: RenewalOutcome,
    transaction: TransactionId,
    journal_seq: u64,
    at_ic: ClockStamp,
    posted_at_utc: i64,
    #[serde(deserialize_with = "required_option")]
    issued_at_ic: Option<ClockStamp>,
}
impl<'de> Deserialize<'de> for RenewalTerminalReceiptV1 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = TerminalWire::deserialize(d)?;
        let value = Self {
            format: w.format,
            commit: w.commit,
            action: w.action,
            outcome: w.outcome,
            transaction: w.transaction,
            journal_seq: w.journal_seq,
            at_ic: w.at_ic,
            posted_at_utc: w.posted_at_utc,
            issued_at_ic: w.issued_at_ic,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl RenewalTerminalReceiptV1 {
    pub fn decode(bytes: &[u8]) -> Result<Self, RenewalAuditError> {
        let row = decode_receipt(bytes)?;
        Self::validate(&row)?;
        Ok(row)
    }
    fn validate(&self) -> Result<(), RenewalAuditError> {
        let id = if self.outcome == RenewalOutcome::Granted {
            GRANT_ACTION
        } else {
            REFUND_ACTION
        };
        if self.format != 1
            || !action_matches(&self.action, id)
            || !versioned(&self.at_ic)
            || self.issued_at_ic.as_ref()
                != (self.outcome == RenewalOutcome::Granted).then_some(&self.at_ic)
            || serde_json::to_vec(self)
                .map_err(|_| RenewalAuditError::Refused)?
                .len()
                > MAX_RENEWAL_RECEIPT_BYTES
        {
            return Err(RenewalAuditError::Refused);
        }
        Ok(())
    }
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn outcome(&self) -> RenewalOutcome {
        self.outcome
    }
    pub const fn transaction(&self) -> TransactionId {
        self.transaction
    }
    pub const fn journal_seq(&self) -> u64 {
        self.journal_seq
    }
    pub fn at_ic(&self) -> &ClockStamp {
        &self.at_ic
    }
    pub const fn posted_at_utc(&self) -> i64 {
        self.posted_at_utc
    }
    pub fn issued_at_ic(&self) -> Option<&ClockStamp> {
        self.issued_at_ic.as_ref()
    }
}
fn decode_receipt<T: serde::de::DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, RenewalAuditError> {
    if bytes.is_empty() || bytes.len() > MAX_RENEWAL_RECEIPT_BYTES {
        return Err(RenewalAuditError::Refused);
    }
    let row: T = serde_json::from_slice(bytes).map_err(|_| RenewalAuditError::Refused)?;
    let original: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| RenewalAuditError::Refused)?;
    if serde_json::to_value(&row).map_err(|_| RenewalAuditError::Refused)? != original {
        return Err(RenewalAuditError::Refused);
    }
    Ok(row)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Proof {
    Fund {
        format: u16,
        operation: CommitToken,
        commit: CommitToken,
        action: DefinitionRef,
        origin: Box<OfferRecord>,
        at_ic: ClockStamp,
    },
    Grant {
        format: u16,
        operation: CommitToken,
        commit: CommitToken,
        action: DefinitionRef,
        at_ic: ClockStamp,
    },
    Refund {
        format: u16,
        operation: CommitToken,
        commit: CommitToken,
        action: DefinitionRef,
        at_ic: ClockStamp,
    },
}
fn encode(proof: &Proof) -> Result<String, ParticipantError> {
    let s = serde_json::to_string(proof).map_err(|_| ParticipantError::Encoding)?;
    if s.len() > MAX_CORRELATION_BYTES {
        return Err(refused());
    }
    Ok(s)
}
pub(super) fn funding_plan(
    origin: &OfferRecord,
    commit: CommitToken,
    action: &DefinitionRef,
    at_ic: &ClockStamp,
) -> Result<EconomyPlan, ParticipantError> {
    origin.validate()?;
    let t = &origin.offer.terms;
    let correlation = encode(&Proof::Fund {
        format: 1,
        operation: origin.operation,
        commit,
        action: action.clone(),
        origin: Box::new(origin.clone()),
        at_ic: at_ic.clone(),
    })?;
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new(NAMESPACE, format!("{}/fund", origin.operation.to_hex())),
        PrincipalId::Object(t.buyer),
        EconomicCause::Ruleset {
            namespace: NAMESPACE.into(),
            correlation: correlation.into(),
        },
        Vec::new(),
    );
    plan.description = "house mind renewal funding".into();
    plan.visibility = EconomicVisibility::Parties;
    plan.hold_ops.push(HoldOp::Place {
        account: t.payer,
        amount: t.price,
        beneficiary: Some(crate::api::contract_principal()),
        purpose: HoldPurpose::Escrow {
            contract: origin.contract,
        },
        expires: None,
    });
    Ok(plan)
}
pub(super) fn terminal_plan(
    history: &RenewalHistory,
    commit: CommitToken,
    action: &DefinitionRef,
    at_ic: &ClockStamp,
    outcome: RenewalOutcome,
) -> Result<EconomyPlan, ParticipantError> {
    let grant = outcome == RenewalOutcome::Granted;
    let t = &history.origin.offer.terms;
    let proof = if grant {
        Proof::Grant {
            format: 1,
            operation: history.operation(),
            commit,
            action: action.clone(),
            at_ic: at_ic.clone(),
        }
    } else {
        Proof::Refund {
            format: 1,
            operation: history.operation(),
            commit,
            action: action.clone(),
            at_ic: at_ic.clone(),
        }
    };
    let mut lines = Vec::new();
    if grant {
        lines.push(RequestedLine {
            account: t.payer,
            delta: t.price.checked_neg().ok_or_else(refused)?,
            memo: format!("contract {} escrow", history.origin.contract).into(),
        });
        for p in t.payouts()? {
            lines.push(RequestedLine {
                account: p.account,
                delta: p.amount,
                memo: format!("contract {} payout", history.origin.contract).into(),
            });
        }
    }
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new(
            NAMESPACE,
            format!(
                "{}/{}",
                history.operation().to_hex(),
                if grant { "grant" } else { "refund" }
            ),
        ),
        PrincipalId::Object(t.buyer),
        EconomicCause::Ruleset {
            namespace: NAMESPACE.into(),
            correlation: encode(&proof)?.into(),
        },
        lines,
    );
    plan.description = format!(
        "{} of contract {}",
        if grant { "settlement" } else { "refund" },
        history.origin.contract
    );
    plan.visibility = EconomicVisibility::Parties;
    if grant {
        plan.expected_hold_revisions
            .push(crate::plan::HoldRevision {
                hold: history.funding.hold,
                revision: history.hold.revision,
            });
    }
    plan.hold_ops.push(if grant {
        HoldOp::Capture {
            hold: history.funding.hold,
            amount: t.price,
        }
    } else {
        HoldOp::Release {
            hold: history.funding.hold,
        }
    });
    Ok(plan)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RenewalAuditError {
    #[error("authoritative renewal replay is incomplete")]
    Incomplete,
    #[error("renewal evidence differs from authoritative history")]
    Refused,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalHistory {
    receipt: RenewalTerminalReceiptV1,
    hold: Hold,
    plan_hash: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenewalHistory {
    origin: OfferRecord,
    funding: RenewalFundingReceiptV1,
    hold: Hold,
    plan_hash: [u8; 32],
    #[serde(deserialize_with = "required_option")]
    terminal: Option<TerminalHistory>,
}
impl RenewalHistory {
    pub fn operation(&self) -> CommitToken {
        self.origin.operation
    }
    pub fn offer(&self) -> &super::RenewalOffer {
        &self.origin.offer
    }
    pub fn funding(&self) -> &RenewalFundingReceiptV1 {
        &self.funding
    }
    pub fn terminal(&self) -> Option<&RenewalTerminalReceiptV1> {
        self.terminal.as_ref().map(|t| &t.receipt)
    }
    pub(super) fn origin(&self) -> &OfferRecord {
        &self.origin
    }
    pub(super) fn contract(&self) -> Result<EconomicContract, RenewalAuditError> {
        let mut c = contract(&self.origin).map_err(|_| RenewalAuditError::Refused)?;
        c.acceptances.push(Acceptance {
            party: PrincipalId::Object(self.origin.offer.terms.buyer),
            at: self.funding.posted_at_utc,
            consent_hash: self.origin.consent_hash,
        });
        c.escrow.push(self.funding.hold);
        c.roster_locked = true;
        c.state = ContractState::ReadyToSettle;
        c.revision = 2;
        if let Some(t) = &self.terminal {
            c.state = if t.receipt.outcome == RenewalOutcome::Granted {
                ContractState::Settled {
                    transaction: t.receipt.transaction,
                }
            } else {
                ContractState::Cancelled
            };
            c.revision = 3;
        }
        Ok(c)
    }
    pub fn verify_documents(
        &self,
        c: &EconomicContract,
        h: &Hold,
    ) -> Result<(), RenewalAuditError> {
        self.validate()?;
        let expected = self.terminal.as_ref().map_or(&self.hold, |t| &t.hold);
        if c != &self.contract()? || h != expected {
            return Err(RenewalAuditError::Refused);
        }
        Ok(())
    }
    /// Conservative future terminal envelope bound, evaluated before a hold is
    /// prepared. It uses the admitted receipt cap plus maximal fixed-width
    /// transaction/sequence values; it creates no receipt or authority.
    pub(super) fn require_terminal_room(&self) -> Result<(), RenewalAuditError> {
        let fail = || RenewalAuditError::Refused;
        if self.terminal.is_some() {
            return Err(fail());
        }
        let mut hold = self.hold.clone();
        hold.status = HoldStatus::Consumed {
            transaction: TransactionId(u64::MAX),
            amount: self.origin.offer.terms.price,
        };
        hold.revision = 2;
        hold.last_journal_seq = u64::MAX;
        let hold_bytes = serde_json::to_vec(&hold).map_err(|_| fail())?.len();
        // {"receipt":...,"hold":...,"plan_hash":[255,...]} including punctuation.
        let fixed = serde_json::to_vec(
            &serde_json::json!({"receipt":null,"hold":null,"plan_hash":vec![255u8;32]}),
        )
        .map_err(|_| fail())?
        .len();
        let terminal_max = fixed
            .checked_sub(8)
            .and_then(|n| n.checked_add(MAX_RENEWAL_RECEIPT_BYTES))
            .and_then(|n| n.checked_add(hold_bytes))
            .ok_or_else(fail)?;
        let current = serde_json::to_vec(self).map_err(|_| fail())?.len();
        let mut contract = self.contract()?;
        contract.state = ContractState::Settled {
            transaction: TransactionId(u64::MAX),
        };
        contract.revision = 3;
        if terminal_max > TERMINAL_RESERVE_BYTES
            || current
                .checked_sub(4)
                .and_then(|n| n.checked_add(terminal_max))
                .is_none_or(|n| n > MAX_HISTORY_BYTES)
            || serde_json::to_vec(&contract).map_err(|_| fail())?.len()
                > super::history::MAX_CONTRACT_BYTES
        {
            return Err(fail());
        }
        Ok(())
    }
    fn validate(&self) -> Result<(), RenewalAuditError> {
        let fail = || RenewalAuditError::Refused;
        self.origin.validate().map_err(|_| fail())?;
        self.funding.validate()?;
        let f = &self.funding;
        let o = &self.origin;
        let terms = &o.offer.terms;
        if o.closure.is_some()
            || [
                o.operation,
                o.offer.basis.predecessor_operation,
                o.offer.basis.prior_quote,
            ]
            .contains(&f.commit)
            || f.contract != o.contract
            || f.consent_hash != o.consent_hash
            || f.at_ic.clock != o.created_at_ic.clock
            || f.at_ic.at < o.created_at_ic.at
            || f.at_ic.at < terms.not_before.at
            || f.at_ic.at >= terms.expires_at.at
            || f.posted_at_utc < o.created_at_utc
        {
            return Err(fail());
        }
        let h = &self.hold;
        if h.id != f.hold
            || h.account != terms.payer
            || h.amount != terms.price
            || h.placed_by != PrincipalId::Object(terms.buyer)
            || h.holder != IdempotencyKey::new(NAMESPACE, format!("{}/fund", o.operation.to_hex()))
            || h.purpose
                != (HoldPurpose::Escrow {
                    contract: o.contract,
                })
            || h.beneficiary != Some(crate::api::contract_principal())
            || h.expires.is_some()
            || h.status != HoldStatus::Held
            || h.revision != 1
            || h.last_journal_seq != f.journal_seq
            || crate::hash::plan_hash(
                &funding_plan(o, f.commit, &f.action, &f.at_ic).map_err(|_| fail())?,
            ) != self.plan_hash
        {
            return Err(fail());
        }
        if let Some(t) = &self.terminal {
            let r = &t.receipt;
            r.validate()?;
            if [
                o.operation,
                o.offer.basis.predecessor_operation,
                o.offer.basis.prior_quote,
                f.commit,
            ]
            .contains(&r.commit)
                || r.transaction == f.transaction
                || r.journal_seq <= f.journal_seq
                || r.posted_at_utc < f.posted_at_utc
            {
                return Err(fail());
            }
            let grant = r.outcome == RenewalOutcome::Granted;
            if grant
                && (r.at_ic.clock != f.at_ic.clock
                    || r.at_ic.at < f.at_ic.at
                    || r.at_ic.at >= terms.expires_at.at)
            {
                return Err(fail());
            }
            let mut expected = h.clone();
            expected.status = if grant {
                HoldStatus::Consumed {
                    transaction: r.transaction,
                    amount: terms.price,
                }
            } else {
                HoldStatus::Released
            };
            expected.revision = 2;
            expected.last_journal_seq = r.journal_seq;
            if t.hold != expected
                || crate::hash::plan_hash(
                    &terminal_plan(self, r.commit, &r.action, &r.at_ic, r.outcome)
                        .map_err(|_| fail())?,
                ) != t.plan_hash
                || serde_json::to_vec(t).map_err(|_| fail())?.len() > TERMINAL_RESERVE_BYTES
            {
                return Err(fail());
            }
        }
        let max = if self.terminal.is_some() {
            MAX_HISTORY_BYTES
        } else {
            MAX_HISTORY_BYTES - TERMINAL_RESERVE_BYTES
        };
        if serde_json::to_vec(self).map_err(|_| fail())?.len() > max
            || serde_json::to_vec(&self.contract()?)
                .map_err(|_| fail())?
                .len()
                > super::history::MAX_CONTRACT_BYTES
        {
            return Err(fail());
        }
        Ok(())
    }
}

fn history_rows<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<CommitToken, RenewalHistory>, D::Error> {
    struct Rows;
    impl<'de> serde::de::Visitor<'de> for Rows {
        type Value = BTreeMap<CommitToken, RenewalHistory>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded distinct renewal histories")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            if map.size_hint().is_some_and(|n| n > MAX_HISTORIES) {
                return Err(serde::de::Error::custom("renewal history capacity"));
            }
            let mut rows = BTreeMap::new();
            while let Some(key) = map.next_key::<CommitToken>()? {
                if rows.len() == MAX_HISTORIES || rows.contains_key(&key) {
                    return Err(serde::de::Error::custom(
                        "renewal history capacity or duplicate",
                    ));
                }
                let row = map.next_value::<RenewalHistory>()?;
                row.validate().map_err(serde::de::Error::custom)?;
                if key != row.operation() {
                    return Err(serde::de::Error::custom("renewal history key"));
                }
                rows.insert(key, row);
            }
            Ok(rows)
        }
    }
    d.deserialize_map(Rows)
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenewalReceiptAudit {
    #[serde(deserialize_with = "history_rows")]
    histories: BTreeMap<CommitToken, RenewalHistory>,
    complete: bool,
    invalid: bool,
    #[serde(skip)]
    verified_this_boot: bool,
}
impl RenewalReceiptAudit {
    pub(crate) fn uninitialized(&self) -> bool {
        !self.complete && !self.invalid && self.histories.is_empty()
    }
    pub fn require_complete(&self) -> Result<(), RenewalAuditError> {
        if !self.complete || !self.verified_this_boot {
            return Err(RenewalAuditError::Incomplete);
        }
        if self.invalid {
            return Err(RenewalAuditError::Refused);
        }
        Ok(())
    }
    pub fn get(&self, operation: CommitToken) -> Result<&RenewalHistory, RenewalAuditError> {
        self.require_complete()?;
        self.histories
            .get(&operation)
            .ok_or(RenewalAuditError::Refused)
    }
    pub(super) fn find(
        &self,
        operation: CommitToken,
    ) -> Result<Option<&RenewalHistory>, RenewalAuditError> {
        self.require_complete()?;
        Ok(self.histories.get(&operation))
    }
    pub fn receipts(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &RenewalHistory>, RenewalAuditError> {
        self.require_complete()?;
        Ok(self.histories.values())
    }
    pub(crate) fn finish_replay(&mut self) -> Result<(), RenewalAuditError> {
        if !self.valid_shape() {
            return Err(RenewalAuditError::Refused);
        }
        self.complete = true;
        self.verified_this_boot = true;
        Ok(())
    }
    fn valid_shape(&self) -> bool {
        !self.invalid
            && self.histories.len() <= MAX_HISTORIES
            && self
                .histories
                .iter()
                .all(|(k, h)| *k == h.operation() && h.validate().is_ok())
    }
    pub(crate) fn checkpoint_complete(&self, rows: &[crate::checkpoint::IdemRow]) -> bool {
        if !self.complete || !self.valid_shape() {
            return false;
        }
        let mut expected = BTreeMap::new();
        for h in self.histories.values() {
            let f = &h.funding;
            expected.insert(
                format!("{}/fund", h.operation().to_hex()),
                (f.transaction, f.journal_seq, f.posted_at_utc, h.plan_hash),
            );
            if let Some(t) = &h.terminal {
                let r = &t.receipt;
                expected.insert(
                    format!(
                        "{}/{}",
                        h.operation().to_hex(),
                        if r.outcome == RenewalOutcome::Granted {
                            "grant"
                        } else {
                            "refund"
                        }
                    ),
                    (r.transaction, r.journal_seq, r.posted_at_utc, t.plan_hash),
                );
            }
        }
        for r in rows.iter().filter(|r| r.ns.as_str() == NAMESPACE) {
            let Some((tx, seq, at, hash)) = expected.remove(r.key.as_str()) else {
                return false;
            };
            let hex: String = hash.iter().map(|v| format!("{v:02x}")).collect();
            if r.tx != tx || r.seq != seq || r.posted_at != at || r.plan_hash != hex {
                return false;
            }
        }
        expected.is_empty()
    }
    pub(crate) fn observe(&mut self, tx: &EconomicTransaction) {
        if tx.idempotency.namespace.as_str() != NAMESPACE
            && !matches!(&tx.cause,EconomicCause::Ruleset{namespace,..}if namespace.as_str()==NAMESPACE)
        {
            return;
        }
        match self.next(tx) {
            Ok(row) => {
                self.histories.insert(row.operation(), row);
            }
            Err(_) => self.invalid = true,
        }
    }
    pub(super) fn prepare(
        &self,
        tx: &EconomicTransaction,
    ) -> Result<PreparedHistory, ParticipantError> {
        self.require_complete().map_err(|_| refused())?;
        let after = self.next(tx).map_err(|_| refused())?;
        Ok(PreparedHistory {
            before: self.histories.get(&after.operation()).cloned(),
            after,
            count: self.histories.len(),
            complete: self.complete,
            invalid: self.invalid,
            verified: self.verified_this_boot,
        })
    }
    fn next(&self, tx: &EconomicTransaction) -> Result<RenewalHistory, RenewalAuditError> {
        let fail = || RenewalAuditError::Refused;
        if self.invalid
            || tx.idempotency.namespace.as_str() != NAMESPACE
            || tx.reverses.is_some()
            || !tx.asset_effects.is_empty()
            || !tx.custody_effects.is_empty()
            || tx.consumed_approval.is_some()
        {
            return Err(fail());
        }
        let EconomicCause::Ruleset {
            namespace,
            correlation,
        } = &tx.cause
        else {
            return Err(fail());
        };
        if namespace.as_str() != NAMESPACE || correlation.len() > MAX_CORRELATION_BYTES {
            return Err(fail());
        }
        let proof: Proof = serde_json::from_str(correlation).map_err(|_| fail())?;
        if encode(&proof).map_err(|_| fail())?.as_str() != correlation.as_str() {
            return Err(fail());
        }
        let [effect] = tx.hold_effects.as_slice() else {
            return Err(fail());
        };
        let row = match proof {
            Proof::Fund {
                format: 1,
                operation,
                commit,
                action,
                origin,
                at_ic,
            } => {
                if self.histories.len() >= MAX_HISTORIES
                    || self.histories.contains_key(&operation)
                    || origin.operation != operation
                    || !tx.lines.is_empty()
                    || effect.kind != HoldEffectKind::Placed
                    || tx.actor != PrincipalId::Object(origin.offer.terms.buyer)
                    || tx.idempotency.key.as_str() != format!("{}/fund", operation.to_hex())
                {
                    return Err(fail());
                }
                RenewalHistory {
                    funding: RenewalFundingReceiptV1 {
                        format: 1,
                        commit,
                        action,
                        contract: origin.contract,
                        hold: effect.hold.id,
                        transaction: tx.id,
                        journal_seq: tx.journal_seq,
                        consent_hash: origin.consent_hash,
                        at_ic,
                        posted_at_utc: tx.posted_at,
                    },
                    origin: *origin,
                    hold: effect.hold.clone(),
                    plan_hash: tx.plan_hash,
                    terminal: None,
                }
            }
            Proof::Grant {
                format: 1,
                operation,
                commit,
                action,
                at_ic,
            } => self.terminal_row(
                tx,
                operation,
                commit,
                action,
                at_ic,
                RenewalOutcome::Granted,
            )?,
            Proof::Refund {
                format: 1,
                operation,
                commit,
                action,
                at_ic,
            } => self.terminal_row(
                tx,
                operation,
                commit,
                action,
                at_ic,
                RenewalOutcome::Refunded,
            )?,
            _ => return Err(fail()),
        };
        row.validate()?;
        Ok(row)
    }
    fn terminal_row(
        &self,
        tx: &EconomicTransaction,
        operation: CommitToken,
        commit: CommitToken,
        action: DefinitionRef,
        at_ic: ClockStamp,
        outcome: RenewalOutcome,
    ) -> Result<RenewalHistory, RenewalAuditError> {
        let fail = || RenewalAuditError::Refused;
        let mut row = self.histories.get(&operation).cloned().ok_or_else(fail)?;
        if row.terminal.is_some() {
            return Err(fail());
        }
        let t = &row.origin.offer.terms;
        let grant = outcome == RenewalOutcome::Granted;
        if tx.actor != PrincipalId::Object(t.buyer)
            || tx.idempotency.key.as_str()
                != format!(
                    "{}/{}",
                    operation.to_hex(),
                    if grant { "grant" } else { "refund" }
                )
        {
            return Err(fail());
        }
        let [effect] = tx.hold_effects.as_slice() else {
            return Err(fail());
        };
        if effect.kind
            != if grant {
                HoldEffectKind::Captured
            } else {
                HoldEffectKind::Released
            }
        {
            return Err(fail());
        }
        let mut actual = Vec::new();
        for line in &tx.lines {
            if line.instrument != t.instrument {
                return Err(fail());
            }
            actual.push((line.account, line.delta.minor()));
        }
        let mut expected = Vec::new();
        if grant {
            expected.push((t.payer, t.price.checked_neg().ok_or_else(fail)?.minor()));
            expected.extend(
                t.payouts()
                    .map_err(|_| fail())?
                    .iter()
                    .map(|p| (p.account, p.amount.minor())),
            );
        }
        actual.sort();
        expected.sort();
        if actual != expected {
            return Err(fail());
        }
        row.terminal = Some(TerminalHistory {
            receipt: RenewalTerminalReceiptV1 {
                format: 1,
                commit,
                action,
                outcome,
                transaction: tx.id,
                journal_seq: tx.journal_seq,
                at_ic: at_ic.clone(),
                posted_at_utc: tx.posted_at,
                issued_at_ic: grant.then_some(at_ic),
            },
            hold: effect.hold.clone(),
            plan_hash: tx.plan_hash,
        });
        Ok(row)
    }
}
pub(super) struct PreparedHistory {
    before: Option<RenewalHistory>,
    after: RenewalHistory,
    count: usize,
    complete: bool,
    invalid: bool,
    verified: bool,
}
impl PreparedHistory {
    pub(super) fn row(&self) -> &RenewalHistory {
        &self.after
    }
    pub(super) fn matches(&self, state: &RenewalReceiptAudit) -> bool {
        state.histories.len() == self.count
            && state.complete == self.complete
            && state.invalid == self.invalid
            && state.verified_this_boot == self.verified
            && state.histories.get(&self.after.operation()) == self.before.as_ref()
    }
    pub(super) fn apply(self, state: &mut RenewalReceiptAudit) {
        state.histories.insert(self.after.operation(), self.after);
    }
}
