//! Exact journal-derived pending escrow history. Serialized rows never open the
//! replay latch; full boot/checkpoint joins must do that before any live read.
use super::plans::{self, FundingIntent, Proof};
use super::{CharterPendingConsentV1, CharterPendingDecisionV1, NAMESPACE, refused};
use crate::contract::{ContractState, EconomicContract};
use crate::record::{
    AcquisitionRecord, AssetEffectKind, AssetLifecycle, EconomicAsset, EconomicCause,
    EconomicTransaction, Hold, HoldEffectKind, HoldPurpose, HoldStatus, IdempotencyKey,
};
use crate::{ContractId, HoldId, TransactionId};
use mush_core::CommitToken;
use mush_core::participant::ParticipantError;
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_HISTORIES: usize = 65_536;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharterPendingFundingReceiptV1 {
    transaction: TransactionId,
    journal_seq: u64,
    plan_hash: [u8; 32],
    hold: Hold,
}
impl CharterPendingFundingReceiptV1 {
    pub const fn transaction(&self) -> TransactionId {
        self.transaction
    }
    pub const fn journal_seq(&self) -> u64 {
        self.journal_seq
    }
    pub const fn hold(&self) -> HoldId {
        self.hold.id
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharterPendingTerminalReceiptV1 {
    decision: CharterPendingDecisionV1,
    transaction: TransactionId,
    journal_seq: u64,
    plan_hash: [u8; 32],
    hold: Hold,
    #[serde(deserialize_with = "required_option")]
    title: Option<EconomicAsset>,
}
impl CharterPendingTerminalReceiptV1 {
    pub fn decision(&self) -> &CharterPendingDecisionV1 {
        &self.decision
    }
    pub const fn transaction(&self) -> TransactionId {
        self.transaction
    }
    pub const fn journal_seq(&self) -> u64 {
        self.journal_seq
    }
    pub fn title(&self) -> Option<&EconomicAsset> {
        self.title.as_ref()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharterPendingHistory {
    intent: FundingIntent,
    funding: CharterPendingFundingReceiptV1,
    #[serde(deserialize_with = "required_option")]
    terminal: Option<CharterPendingTerminalReceiptV1>,
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
impl CharterPendingHistory {
    pub fn consent(&self) -> &CharterPendingConsentV1 {
        &self.intent.consent
    }
    pub const fn contract_id(&self) -> ContractId {
        self.intent.contract
    }
    pub fn funding(&self) -> &CharterPendingFundingReceiptV1 {
        &self.funding
    }
    pub fn terminal(&self) -> Option<&CharterPendingTerminalReceiptV1> {
        self.terminal.as_ref()
    }
    pub(super) fn intent(&self) -> &FundingIntent {
        &self.intent
    }
    pub(crate) fn issuer(&self) -> &mush_org::SystemPrincipalId {
        &self.intent.issuer
    }
    pub(super) fn hold(&self) -> &Hold {
        self.terminal
            .as_ref()
            .map_or(&self.funding.hold, |t| &t.hold)
    }
    pub fn decode(raw: &[u8]) -> Result<Self, ParticipantError> {
        if raw.len() > super::MAX_PENDING_HISTORY_BYTES {
            return Err(refused());
        }
        let row: Self = serde_json::from_slice(raw).map_err(|_| refused())?;
        row.validate()?;
        if bounded(&row, super::MAX_PENDING_HISTORY_BYTES)? != raw {
            return Err(refused());
        }
        Ok(row)
    }
    pub(super) fn contract(&self) -> Result<EconomicContract, ParticipantError> {
        let mut c =
            super::contract::funded(&self.intent.consent, self.intent.contract, self.intent.hold)?;
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
        bounded(&c, super::MAX_PENDING_CONTRACT_BYTES)?;
        Ok(c)
    }
    pub fn verify_documents(
        &self,
        contract: &EconomicContract,
        hold: &Hold,
    ) -> Result<(), ParticipantError> {
        self.validate()?;
        if contract != &self.contract()? || hold != self.hold() {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn validate(&self) -> Result<(), ParticipantError> {
        self.intent.validate()?;
        let consent = &self.intent.consent;
        let f = &self.funding;
        let h = &f.hold;
        if h.id != self.intent.hold
            || h.account != consent.payer
            || h.amount != consent.terms.price
            || h.status != HoldStatus::Held
            || h.placed_by != PrincipalId::Object(consent.terms.buyer)
            || h.beneficiary != Some(crate::api::contract_principal())
            || h.purpose
                != (HoldPurpose::Escrow {
                    contract: self.intent.contract,
                })
            || h.holder
                != IdempotencyKey::new(NAMESPACE, format!("{}/fund", consent.operation.to_hex()))
            || h.expires.is_some()
            || h.revision != 1
            || h.last_journal_seq != f.journal_seq
            || f.plan_hash != crate::hash::plan_hash(&plans::funding(&self.intent)?)
        {
            return Err(refused());
        }
        if let Some(t) = &self.terminal {
            t.decision.matches_consent(consent)?;
            let settle = t.decision.reason.settles();
            let mut expected = h.clone();
            expected.status = if settle {
                HoldStatus::Consumed {
                    transaction: t.transaction,
                    amount: consent.terms.price,
                }
            } else {
                HoldStatus::Released
            };
            expected.revision = 2;
            expected.last_journal_seq = t.journal_seq;
            let expected_title = if settle {
                Some(title_after(
                    &self.intent,
                    t.transaction,
                    t.journal_seq,
                    t.decision.posted_at_utc,
                )?)
            } else {
                None
            };
            if t.transaction == f.transaction
                || t.journal_seq <= f.journal_seq
                || t.hold != expected
                || t.title != expected_title
                || t.plan_hash
                    != crate::hash::plan_hash(&plans::terminal(&self.intent, &t.decision)?)
            {
                return Err(refused());
            }
        }
        bounded(self, super::MAX_PENDING_HISTORY_BYTES)?;
        self.contract()?;
        Ok(())
    }
    /// Every fund preflights complete fixed-shape terminal documents. The
    /// synthetic maximum values are sizing only and never become authority.
    pub(super) fn require_terminal_room(&self) -> Result<(), ParticipantError> {
        if self.terminal.is_some() {
            return Err(refused());
        }
        let mut h = self.funding.hold.clone();
        h.status = HoldStatus::Consumed {
            transaction: TransactionId(u64::MAX),
            amount: self.intent.consent.terms.price,
        };
        h.revision = 2;
        h.last_journal_seq = u64::MAX;
        let title = title_after(&self.intent, TransactionId(u64::MAX), u64::MAX, i64::MIN)?;
        // The exact full action ref and tagged decision are bounded on admission
        // at 2048 bytes. Include punctuation and both required nullable slots.
        let envelope = serde_json::json!({"decision":null,"transaction":u64::MAX,
            "journal_seq":u64::MAX,"plan_hash":vec![255u8;32],"hold":h,"title":title});
        let terminal_max = bounded(&envelope, super::MAX_PENDING_HISTORY_BYTES)?
            .len()
            .checked_sub(4)
            .and_then(|n| n.checked_add(2048))
            .ok_or_else(refused)?;
        let present = bounded(self, super::MAX_PENDING_HISTORY_BYTES)?.len();
        if present
            .checked_sub(4)
            .and_then(|n| n.checked_add(terminal_max))
            .is_none_or(|n| n > super::MAX_PENDING_HISTORY_BYTES)
        {
            return Err(refused());
        }
        let mut c = self.contract()?;
        c.state = ContractState::Settled {
            transaction: TransactionId(u64::MAX),
        };
        c.revision = 2;
        bounded(&c, super::MAX_PENDING_CONTRACT_BYTES)?;
        Ok(())
    }
}
fn bounded<T: Serialize>(value: &T, max: usize) -> Result<Vec<u8>, ParticipantError> {
    mush_core::canonical_json_bytes_with_limit(
        &serde_json::to_value(value).map_err(|_| refused())?,
        max,
    )
    .map_err(|_| refused())
}
fn title_after(
    intent: &FundingIntent,
    tx: TransactionId,
    seq: u64,
    at: i64,
) -> Result<EconomicAsset, ParticipantError> {
    Ok(EconomicAsset {
        object: intent.consent.terms.charter,
        title_owner: PrincipalId::Object(intent.consent.terms.buyer),
        scope: None,
        assignment: None,
        maintenance_account: None,
        catalog: Some(intent.catalog.clone()),
        encumbrances: Vec::new(),
        acquisition: Some(AcquisitionRecord {
            transaction: tx,
            at,
        }),
        lifecycle: AssetLifecycle::Active,
        last_journal_seq: seq,
        revision: intent
            .consent
            .terms
            .title_revision
            .checked_add(1)
            .ok_or_else(refused)?,
        stolen: None,
        frozen: false,
        frozen_reason: None,
    })
}
fn transaction_matches(
    tx: &EconomicTransaction,
    plan: &crate::EconomyPlan,
    intent: &FundingIntent,
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
            .any(|l| l.instrument != intent.consent.terms.instrument)
    {
        return Err(refused());
    }
    Ok(())
}
fn from_funding(
    tx: &EconomicTransaction,
    intent: FundingIntent,
) -> Result<CharterPendingHistory, ParticipantError> {
    intent.validate()?;
    transaction_matches(tx, &plans::funding(&intent)?, &intent)?;
    let [hold] = tx.hold_effects.as_slice() else {
        return Err(refused());
    };
    if hold.kind != HoldEffectKind::Placed
        || !tx.asset_effects.is_empty()
        || tx.posted_at != intent.consent.accepted_at_utc
    {
        return Err(refused());
    }
    let row = CharterPendingHistory {
        intent,
        funding: CharterPendingFundingReceiptV1 {
            transaction: tx.id,
            journal_seq: tx.journal_seq,
            plan_hash: tx.plan_hash,
            hold: hold.hold.clone(),
        },
        terminal: None,
    };
    row.validate()?;
    row.require_terminal_room()?;
    Ok(row)
}
fn with_terminal(
    before: &CharterPendingHistory,
    tx: &EconomicTransaction,
    decision: CharterPendingDecisionV1,
) -> Result<CharterPendingHistory, ParticipantError> {
    before.validate()?;
    if before.terminal.is_some() || tx.posted_at != decision.posted_at_utc {
        return Err(refused());
    }
    transaction_matches(
        tx,
        &plans::terminal(&before.intent, &decision)?,
        &before.intent,
    )?;
    let [hold] = tx.hold_effects.as_slice() else {
        return Err(refused());
    };
    let settle = decision.reason.settles();
    let title = if settle {
        let [effect] = tx.asset_effects.as_slice() else {
            return Err(refused());
        };
        if effect.object != before.consent().terms.charter
            || effect.kind != AssetEffectKind::TitleTransferred
        {
            return Err(refused());
        }
        Some(effect.post.clone())
    } else {
        if !tx.asset_effects.is_empty() {
            return Err(refused());
        }
        None
    };
    if hold.kind
        != (if settle {
            HoldEffectKind::Captured
        } else {
            HoldEffectKind::Released
        })
    {
        return Err(refused());
    }
    let mut row = before.clone();
    row.terminal = Some(CharterPendingTerminalReceiptV1 {
        decision,
        transaction: tx.id,
        journal_seq: tx.journal_seq,
        plan_hash: tx.plan_hash,
        hold: hold.hold.clone(),
        title,
    });
    row.validate()?;
    Ok(row)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CharterPendingAudit {
    #[serde(deserialize_with = "rows")]
    histories: BTreeMap<CommitToken, CharterPendingHistory>,
    complete: bool,
    invalid: bool,
    #[serde(skip)]
    verified: bool,
}
fn rows<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<CommitToken, CharterPendingHistory>, D::Error> {
    struct Rows;
    impl<'de> serde::de::Visitor<'de> for Rows {
        type Value = BTreeMap<CommitToken, CharterPendingHistory>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded distinct charter pending histories")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            if map.size_hint().is_some_and(|n| n > MAX_HISTORIES) {
                return Err(serde::de::Error::custom("pending history limit"));
            }
            let mut rows = BTreeMap::new();
            while let Some(key) = map.next_key::<CommitToken>()? {
                if rows.len() == MAX_HISTORIES || rows.contains_key(&key) {
                    return Err(serde::de::Error::custom(
                        "pending history duplicate or limit",
                    ));
                }
                let row = map.next_value::<CharterPendingHistory>()?;
                row.validate().map_err(serde::de::Error::custom)?;
                if key != row.consent().operation {
                    return Err(serde::de::Error::custom("pending history key"));
                }
                rows.insert(key, row);
            }
            Ok(rows)
        }
    }
    d.deserialize_map(Rows)
}
impl CharterPendingAudit {
    pub(super) fn prepare_patch(
        &self,
        tx: &EconomicTransaction,
    ) -> Result<PreparedHistory, ParticipantError> {
        let after = self.prepare(tx)?;
        Ok(PreparedHistory {
            before: self.find(after.consent().operation)?.cloned(),
            count: self.histories.len(),
            after,
        })
    }
    pub(crate) fn uninitialized(&self) -> bool {
        !self.complete && !self.invalid && self.histories.is_empty()
    }
    pub(crate) fn require_complete(&self) -> Result<(), ParticipantError> {
        if !self.complete || !self.verified || self.invalid {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn get(&self, op: CommitToken) -> Result<&CharterPendingHistory, ParticipantError> {
        self.require_complete()?;
        self.histories.get(&op).ok_or_else(refused)
    }
    pub(crate) fn histories(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &CharterPendingHistory>, ParticipantError> {
        self.require_complete()?;
        Ok(self.histories.values())
    }
    pub(super) fn find(
        &self,
        op: CommitToken,
    ) -> Result<Option<&CharterPendingHistory>, ParticipantError> {
        self.require_complete()?;
        Ok(self.histories.get(&op))
    }
    pub(crate) fn pending_settlement(
        &self,
        tx: &EconomicTransaction,
    ) -> Result<&CharterPendingHistory, ParticipantError> {
        if self.invalid || tx.idempotency.namespace.as_str() != NAMESPACE {
            return Err(refused());
        }
        let EconomicCause::Ruleset {
            namespace,
            correlation,
        } = &tx.cause
        else {
            return Err(refused());
        };
        if namespace.as_str() != NAMESPACE {
            return Err(refused());
        }
        let Proof::Terminal {
            format: 1,
            funding,
            decision,
        } = plans::decode(correlation.as_str())?
        else {
            return Err(refused());
        };
        let row = self.histories.get(&funding).ok_or_else(refused)?;
        let terminal = row.terminal.as_ref().ok_or_else(refused)?;
        if !decision.reason.settles()
            || terminal.decision != decision
            || terminal.transaction != tx.id
            || terminal.journal_seq != tx.journal_seq
            || terminal.plan_hash != tx.plan_hash
        {
            return Err(refused());
        }
        row.validate()?;
        Ok(row)
    }
    pub(crate) fn require_absent(&self) -> Result<(), ParticipantError> {
        self.require_complete()?;
        if !self.histories.is_empty() {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn finish_replay(&mut self) -> Result<(), ParticipantError> {
        self.validate()?;
        self.complete = true;
        self.verified = true;
        Ok(())
    }
    fn validate(&self) -> Result<(), ParticipantError> {
        if self.invalid || self.histories.len() > MAX_HISTORIES {
            return Err(refused());
        }
        let mut contracts = BTreeSet::new();
        let mut holds = BTreeSet::new();
        let mut commits = BTreeSet::new();
        for (key, row) in &self.histories {
            row.validate()?;
            if *key != row.consent().operation
                || !contracts.insert(row.contract_id())
                || !holds.insert(row.intent.hold)
                || !commits.insert(*key)
            {
                return Err(refused());
            }
            if let Some(t) = &row.terminal {
                if !commits.insert(t.decision.operation) {
                    return Err(refused());
                }
            }
        }
        Ok(())
    }
    pub(crate) fn checkpoint_complete(&self, index: &[crate::checkpoint::IdemRow]) -> bool {
        if self.uninitialized() {
            return !index.iter().any(|r| r.ns.as_str() == NAMESPACE);
        }
        if !self.complete || self.validate().is_err() {
            return false;
        }
        let mut expected = BTreeMap::new();
        for row in self.histories.values() {
            let op = row.consent().operation.to_hex();
            let f = &row.funding;
            expected.insert(
                format!("{op}/fund"),
                (
                    f.transaction,
                    f.journal_seq,
                    row.consent().accepted_at_utc,
                    f.plan_hash,
                ),
            );
            if let Some(t) = &row.terminal {
                expected.insert(
                    format!(
                        "{op}/{}",
                        if t.decision.reason.settles() {
                            "settle"
                        } else {
                            "refund"
                        }
                    ),
                    (
                        t.transaction,
                        t.journal_seq,
                        t.decision.posted_at_utc,
                        t.plan_hash,
                    ),
                );
            }
        }
        for r in index.iter().filter(|r| r.ns.as_str() == NAMESPACE) {
            let Some((tx, seq, at, hash)) = expected.remove(r.key.as_str()) else {
                return false;
            };
            let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
            if r.tx != tx || r.seq != seq || r.posted_at != at || r.plan_hash != hex {
                return false;
            }
        }
        expected.is_empty()
    }
    pub(super) fn prepare(
        &self,
        tx: &EconomicTransaction,
    ) -> Result<CharterPendingHistory, ParticipantError> {
        self.require_complete()?;
        self.next(tx)
    }
    fn next(&self, tx: &EconomicTransaction) -> Result<CharterPendingHistory, ParticipantError> {
        if tx.idempotency.namespace.as_str() != NAMESPACE {
            return Err(refused());
        }
        let EconomicCause::Ruleset {
            namespace,
            correlation,
        } = &tx.cause
        else {
            return Err(refused());
        };
        if namespace.as_str() != NAMESPACE {
            return Err(refused());
        }
        match plans::decode(correlation.as_str())? {
            Proof::Fund { format: 1, intent } => {
                if self.histories.len() == MAX_HISTORIES
                    || self.histories.contains_key(&intent.consent.operation)
                {
                    return Err(refused());
                }
                from_funding(tx, intent)
            }
            Proof::Terminal {
                format: 1,
                funding,
                decision,
            } => with_terminal(
                self.histories.get(&funding).ok_or_else(refused)?,
                tx,
                decision,
            ),
            _ => Err(refused()),
        }
    }
    pub(super) fn install(&mut self, row: CharterPendingHistory) {
        self.histories.insert(row.consent().operation, row);
    }
    pub(crate) fn observe(&mut self, tx: &EconomicTransaction) {
        if tx.idempotency.namespace.as_str() != NAMESPACE {
            return;
        }
        match self.next(tx) {
            Ok(row) => self.install(row),
            Err(_) => self.invalid = true,
        }
    }
}

pub(super) struct PreparedHistory {
    before: Option<CharterPendingHistory>,
    count: usize,
    after: CharterPendingHistory,
}
impl PreparedHistory {
    pub(super) fn row(&self) -> &CharterPendingHistory {
        &self.after
    }
    pub(super) fn matches(&self, state: &CharterPendingAudit) -> bool {
        state.require_complete().is_ok()
            && state.histories.len() == self.count
            && state.histories.get(&self.after.consent().operation) == self.before.as_ref()
    }
    pub(super) fn apply(self, state: &mut CharterPendingAudit) {
        state.install(self.after);
    }
}

#[cfg(test)]
#[path = "history_capacity_tests.rs"]
mod capacity_tests;
