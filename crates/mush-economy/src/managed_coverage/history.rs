//! Exact native journal history. No serialized field opens the replay latch;
//! only the full journal/checkpoint and live document join can do that.
use super::{
    plans::{self, BindingIntent},
    *,
};
use crate::contract::{ContractState, EconomicContract};
use crate::record::{
    AcquisitionRecord, AssetEffectKind, EconomicAsset, EconomicTransaction, Encumbrance, Hold,
    HoldEffectKind, HoldPurpose, HoldStatus, IdempotencyKey,
};
use crate::{ContractId, HoldId, TransactionId};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageBindingReceiptV1 {
    transaction: TransactionId,
    journal_seq: u64,
    plan_hash: [u8; 32],
    hold: Hold,
    title: EconomicAsset,
}
impl CoverageBindingReceiptV1 {
    pub const fn transaction(&self) -> TransactionId {
        self.transaction
    }
    pub const fn journal_seq(&self) -> u64 {
        self.journal_seq
    }
    pub const fn plan_hash(&self) -> [u8; 32] {
        self.plan_hash
    }
    pub const fn hold(&self) -> HoldId {
        self.hold.id
    }
    pub fn title(&self) -> &EconomicAsset {
        &self.title
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageTerminalReceiptV1 {
    decision: CoverageDecisionV1,
    before_title: EconomicAsset,
    transaction: TransactionId,
    journal_seq: u64,
    plan_hash: [u8; 32],
    hold: Hold,
    title: EconomicAsset,
}
impl CoverageTerminalReceiptV1 {
    pub const fn plan_hash(&self) -> [u8; 32] {
        self.plan_hash
    }
    pub(super) fn before_title(&self) -> &EconomicAsset {
        &self.before_title
    }
    pub fn decision(&self) -> &CoverageDecisionV1 {
        &self.decision
    }
    pub const fn transaction(&self) -> TransactionId {
        self.transaction
    }
    pub const fn journal_seq(&self) -> u64 {
        self.journal_seq
    }
    pub fn title(&self) -> &EconomicAsset {
        &self.title
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageHistory {
    intent: BindingIntent,
    binding: Box<CoverageBindingReceiptV1>,
    #[serde(deserialize_with = "required_option")]
    terminal: Option<Box<CoverageTerminalReceiptV1>>,
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
fn bounded<T: Serialize>(row: &T, limit: usize) -> Result<Vec<u8>, ParticipantError> {
    mush_core::canonical_json_bytes_with_limit(
        &serde_json::to_value(row).map_err(|_| refused())?,
        limit,
    )
    .map_err(|_| refused())
}
impl CoverageHistory {
    pub fn consent(&self) -> &CoverageConsentV1 {
        &self.intent.consent
    }
    pub const fn contract_id(&self) -> ContractId {
        self.intent.contract
    }
    pub fn binding(&self) -> &CoverageBindingReceiptV1 {
        &self.binding
    }
    pub fn terminal(&self) -> Option<&CoverageTerminalReceiptV1> {
        self.terminal.as_deref()
    }
    pub(super) fn intent(&self) -> &BindingIntent {
        &self.intent
    }
    pub(super) fn hold(&self) -> &Hold {
        self.terminal
            .as_ref()
            .map_or(&self.binding.hold, |t| &t.hold)
    }
    pub fn decode(raw: &[u8]) -> Result<Self, ParticipantError> {
        if raw.is_empty() || raw.len() > MAX_HISTORY_BYTES {
            return Err(refused());
        }
        let row: Self = serde_json::from_slice(raw).map_err(|_| refused())?;
        row.validate()?;
        if bounded(&row, MAX_HISTORY_BYTES)? != raw {
            return Err(refused());
        }
        Ok(row)
    }
    pub fn digest(&self) -> Result<mush_core::DefinitionDigest, ParticipantError> {
        self.validate()?;
        mush_core::DefinitionDigest::for_canonical_value_with_limit(
            "economy.coverage.history.v1",
            &serde_json::to_value(self).map_err(|_| refused())?,
            MAX_HISTORY_BYTES,
        )
        .map_err(|_| refused())
    }
    pub(super) fn contract(&self) -> Result<EconomicContract, ParticipantError> {
        let mut c =
            super::contract::active(self.consent(), self.intent.contract, self.intent.hold)?;
        if let Some(t) = &self.terminal {
            // Native managed collateral fences ordinary title operations. A
            // changed title cannot be smuggled into the settlement receipt.
            if t.before_title != self.binding.title {
                return Err(refused());
            }
            c.state = if t.decision.reason.pays() {
                ContractState::Settled {
                    transaction: t.transaction,
                }
            } else if matches!(
                t.decision.reason,
                CoverageDecisionKindV1::ExpiredClean { .. }
            ) {
                ContractState::Expired
            } else {
                ContractState::Cancelled
            };
            c.revision = 2;
        }
        bounded(&c, MAX_CONTRACT_BYTES)?;
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
        let c = self.consent();
        let b = &self.binding;
        let h = &b.hold;
        if h.id != self.intent.hold
            || h.account != c.terms.treasury
            || h.amount != c.terms.liability
            || h.status != HoldStatus::Held
            || h.placed_by != PrincipalId::Object(c.terms.buyer)
            || h.beneficiary != Some(crate::api::contract_principal())
            || h.purpose
                != (HoldPurpose::Escrow {
                    contract: self.intent.contract,
                })
            || h.holder != IdempotencyKey::new(NAMESPACE, format!("{}/bind", c.operation.to_hex()))
            || h.expires.is_some()
            || h.revision != 1
            || h.last_journal_seq != b.journal_seq
            || b.plan_hash != crate::hash::plan_hash(&plans::bind(&self.intent)?)
            || b.title != binding_title(&self.intent, b.journal_seq)?
        {
            return Err(refused());
        }
        if let Some(t) = &self.terminal {
            t.decision.matches_consent(c)?;
            let plan = plans::terminal(&self.intent, &t.decision, &t.before_title)?;
            let mut expected = h.clone();
            expected.revision = 2;
            expected.last_journal_seq = t.journal_seq;
            expected.status = if t.decision.reason.pays() {
                HoldStatus::Consumed {
                    transaction: t.transaction,
                    amount: c.terms.liability,
                }
            } else {
                HoldStatus::Released
            };
            let (_, title) = terminal_titles(
                &t.before_title,
                t.decision.reason.pays(),
                c,
                t.transaction,
                t.journal_seq,
                t.decision.posted_at_utc,
            )?;
            if t.transaction == b.transaction
                || t.journal_seq <= b.journal_seq
                || t.hold != expected
                || t.plan_hash != crate::hash::plan_hash(&plan)
                || t.title != title
            {
                return Err(refused());
            }
        }
        bounded(self, MAX_HISTORY_BYTES)?;
        self.contract()?;
        Ok(())
    }
    pub(super) fn require_terminal_room(&self) -> Result<(), ParticipantError> {
        if self.terminal.is_some() {
            return Err(refused());
        }
        // Freeze the exact title while covered, and preflight complete future
        // images at maximum numeric widths before the premium can be posted.
        let mut maximum = self.binding.title.clone();
        maximum.last_journal_seq = u64::MAX;
        maximum.revision = u64::MAX - 2;
        bounded(&maximum, MAX_TITLE_BYTES)?;
        let (released, claimed) = terminal_titles(
            &maximum,
            true,
            self.consent(),
            TransactionId(u64::MAX),
            u64::MAX,
            i64::MIN,
        )?;
        bounded(&released, MAX_TITLE_BYTES)?;
        bounded(&claimed, MAX_TITLE_BYTES)?;
        // Exact admitted bounds include both complete title images and decision.
        // The native hold has fixed fields, whose numeric terminal widths are
        // preflighted here; no authority is created by this sizing envelope.
        let mut hold = self.binding.hold.clone();
        hold.status = HoldStatus::Consumed {
            transaction: TransactionId(u64::MAX),
            amount: self.consent().terms.liability,
        };
        hold.revision = 2;
        hold.last_journal_seq = u64::MAX;
        let envelope = serde_json::json!({"decision":null,"before_title":null,
            "transaction":u64::MAX,"journal_seq":u64::MAX,"plan_hash":vec![255u8;32],
            "hold":hold,"title":null});
        let terminal_max = bounded(&envelope, MAX_HISTORY_BYTES)?
            .len()
            .checked_sub(12)
            .and_then(|n| n.checked_add(MAX_DECISION_BYTES + 2 * MAX_TITLE_BYTES))
            .ok_or_else(refused)?;
        let current = bounded(self, MAX_HISTORY_BYTES)?.len();
        if current
            .checked_sub(4)
            .and_then(|n| n.checked_add(terminal_max))
            .is_none_or(|n| n > MAX_HISTORY_BYTES)
        {
            return Err(refused());
        }
        let mut contract = self.contract()?;
        contract.state = ContractState::Settled {
            transaction: TransactionId(u64::MAX),
        };
        contract.revision = 2;
        bounded(&contract, MAX_CONTRACT_BYTES)?;
        Ok(())
    }
}
fn binding_title(intent: &BindingIntent, seq: u64) -> Result<EconomicAsset, ParticipantError> {
    let mut title = intent.title.clone();
    title.encumbrances.push(Encumbrance::Collateral {
        contract: intent.contract,
    });
    title.revision = title.revision.checked_add(1).ok_or_else(refused)?;
    title.last_journal_seq = seq;
    Ok(title)
}
fn terminal_titles(
    before: &EconomicAsset,
    pays: bool,
    c: &CoverageConsentV1,
    tx: TransactionId,
    seq: u64,
    at: i64,
) -> Result<(EconomicAsset, EconomicAsset), ParticipantError> {
    let mut released = before.clone();
    released.encumbrances.clear();
    released.revision = released.revision.checked_add(1).ok_or_else(refused)?;
    released.last_journal_seq = seq;
    let mut after = released.clone();
    if pays {
        after.title_owner = PrincipalId::Organization(c.terms.insurer);
        after.scope = Some(c.terms.scope);
        after.assignment = None;
        after.maintenance_account = None;
        after.acquisition = Some(AcquisitionRecord {
            transaction: tx,
            at,
        });
        after.revision = after.revision.checked_add(1).ok_or_else(refused)?;
    }
    Ok((released, after))
}
fn transaction_matches(
    tx: &EconomicTransaction,
    plan: &crate::EconomyPlan,
    intent: &BindingIntent,
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
pub(super) fn from_binding(
    tx: &EconomicTransaction,
    intent: BindingIntent,
) -> Result<CoverageHistory, ParticipantError> {
    intent.validate()?;
    transaction_matches(tx, &plans::bind(&intent)?, &intent)?;
    let [hold] = tx.hold_effects.as_slice() else {
        return Err(refused());
    };
    let [title] = tx.asset_effects.as_slice() else {
        return Err(refused());
    };
    if hold.kind != HoldEffectKind::Placed
        || title.kind != AssetEffectKind::Encumbered
        || title.object != intent.consent.basis.hull
        || tx.posted_at != intent.consent.accepted_at_utc
    {
        return Err(refused());
    }
    let row = CoverageHistory {
        intent,
        binding: Box::new(CoverageBindingReceiptV1 {
            transaction: tx.id,
            journal_seq: tx.journal_seq,
            plan_hash: tx.plan_hash,
            hold: hold.hold.clone(),
            title: title.post.clone(),
        }),
        terminal: None,
    };
    row.validate()?;
    row.require_terminal_room()?;
    Ok(row)
}
pub(super) fn with_terminal(
    before: &CoverageHistory,
    tx: &EconomicTransaction,
    decision: CoverageDecisionV1,
    title: EconomicAsset,
) -> Result<CoverageHistory, ParticipantError> {
    before.validate()?;
    if before.terminal.is_some()
        || tx.posted_at != decision.posted_at_utc
        || title != before.binding.title
    {
        return Err(refused());
    }
    transaction_matches(
        tx,
        &plans::terminal(&before.intent, &decision, &title)?,
        &before.intent,
    )?;
    let [hold] = tx.hold_effects.as_slice() else {
        return Err(refused());
    };
    let pays = decision.reason.pays();
    if hold.kind
        != if pays {
            HoldEffectKind::Captured
        } else {
            HoldEffectKind::Released
        }
    {
        return Err(refused());
    }
    let (released, final_title) = terminal_titles(
        &title,
        pays,
        before.consent(),
        tx.id,
        tx.journal_seq,
        tx.posted_at,
    )?;
    let expected = if pays {
        vec![
            (AssetEffectKind::Released, &released),
            (AssetEffectKind::TitleTransferred, &final_title),
        ]
    } else {
        vec![(AssetEffectKind::Released, &released)]
    };
    if tx.asset_effects.len() != expected.len()
        || tx
            .asset_effects
            .iter()
            .zip(expected)
            .any(|(a, (kind, post))| {
                a.object != before.consent().basis.hull || a.kind != kind || &a.post != post
            })
    {
        return Err(refused());
    }
    let mut row = before.clone();
    row.terminal = Some(Box::new(CoverageTerminalReceiptV1 {
        decision,
        before_title: title,
        transaction: tx.id,
        journal_seq: tx.journal_seq,
        plan_hash: tx.plan_hash,
        hold: hold.hold.clone(),
        title: final_title,
    }));
    row.validate()?;
    Ok(row)
}
