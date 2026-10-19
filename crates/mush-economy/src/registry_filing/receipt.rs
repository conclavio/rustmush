//! Journal-derived proof of the closed initial-filing effect.
//!
//! These rows are folded from actual immutable transactions and carried inside
//! the hashed economy checkpoint. A current title is deliberately not evidence
//! that the original fee/title transaction occurred.

use std::collections::BTreeMap;

use super::house::{self, HouseFilingAttribution};
use mush_core::{ObjId, action::CommitToken};
use mush_org::{PrincipalId, SystemPrincipalId};
use serde::{Deserialize, Serialize};

use crate::amount::Amount;
use crate::ids::{AccountId, TransactionId};
use crate::record::{AssetEffectKind, CatalogRef, EconomicCause, EconomicTransaction};

pub(super) const FILING_NAMESPACE: &str = "roll.filing.v1";
/// A permanent receipt is charged against the existing Roll document bound.
/// The producer refuses before publication when this journal-side bound is full.
pub(super) const MAX_FILING_RECEIPTS: usize = 65_536;

/// A comparison subject, never an authorization or proof constructor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialFilingAuditSubject {
    commit: CommitToken,
    transaction: TransactionId,
    journal_seq: u64,
    title: ObjId,
    issued_owner: PrincipalId,
    title_revision: u64,
    payer: AccountId,
    sink: AccountId,
    fee: Amount,
    issuer: SystemPrincipalId,
    catalog: CatalogRef,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_house"
    )]
    house: Option<HouseFilingAttribution>,
}

fn present_house<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<HouseFilingAttribution>, D::Error> {
    HouseFilingAttribution::deserialize(deserializer).map(Some)
}

impl InitialFilingAuditSubject {
    pub fn house(&self) -> Option<&HouseFilingAttribution> {
        self.house.as_ref()
    }
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub(super) const fn commit_token(&self) -> CommitToken {
        self.commit
    }
    /// Capture the exact fields retained by the typed operation receipt. This
    /// checks shape only; only `ReceiptAudit::verify` consults journal evidence.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        commit: CommitToken,
        transaction: TransactionId,
        journal_seq: u64,
        title: ObjId,
        issued_owner: PrincipalId,
        title_revision: u64,
        payer: AccountId,
        sink: AccountId,
        fee: Amount,
        issuer: SystemPrincipalId,
        catalog: CatalogRef,
    ) -> Result<Self, ReceiptAuditError> {
        if title.0 == 0
            || title_revision != 1
            || payer == sink
            || fee.is_negative()
            || !matches!(issued_owner, PrincipalId::Object(_))
        {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(Self {
            commit,
            transaction,
            journal_seq,
            title,
            issued_owner,
            title_revision,
            payer,
            sink,
            fee,
            issuer,
            catalog,
            house: None,
        })
    }
    /// Exact scoped counterpart. It remains a comparison value, never proof.
    #[allow(clippy::too_many_arguments)]
    pub fn new_house(
        commit: CommitToken,
        transaction: TransactionId,
        journal_seq: u64,
        title: ObjId,
        title_revision: u64,
        payer: AccountId,
        sink: AccountId,
        fee: Amount,
        issuer: SystemPrincipalId,
        catalog: CatalogRef,
        house: HouseFilingAttribution,
    ) -> Result<Self, ReceiptAuditError> {
        let mut subject = Self::new(
            commit,
            transaction,
            journal_seq,
            title,
            PrincipalId::Object(house.curator()),
            title_revision,
            payer,
            sink,
            fee,
            issuer,
            catalog,
        )?;
        subject.issued_owner = PrincipalId::Organization(house.organization());
        subject.house = Some(house);
        Ok(subject)
    }
    fn valid(&self) -> bool {
        self.title.0 != 0
            && self.title_revision == 1
            && self.payer != self.sink
            && !self.fee.is_negative()
            && match &self.house {
                None => matches!(self.issued_owner, PrincipalId::Object(_)),
                Some(house) => self.issued_owner == PrincipalId::Organization(house.organization()),
            }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ReceiptAuditError {
    #[error("the authoritative filing receipt replay is incomplete")]
    Incomplete,
    #[error("the retained filing receipt does not match authoritative history")]
    Refused,
}

/// Private provenance is established only by complete replay or a validated
/// checkpoint plus its complete tail. Deserialization alone never opens it.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptAudit {
    receipts: BTreeMap<CommitToken, InitialFilingAuditSubject>,
    complete: bool,
    invalid: bool,
    #[serde(skip)]
    verified_this_boot: bool,
}

impl ReceiptAudit {
    pub(crate) fn uninitialized(&self) -> bool {
        !self.complete && !self.invalid && self.receipts.is_empty()
    }

    pub(super) fn install_prepared(&mut self, subject: InitialFilingAuditSubject) {
        self.receipts.insert(subject.commit, subject);
    }
    pub fn require_complete(&self) -> Result<(), ReceiptAuditError> {
        if !self.complete || !self.verified_this_boot {
            return Err(ReceiptAuditError::Incomplete);
        }
        if self.invalid {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(())
    }

    /// Empty is meaningful only after complete authoritative replay.
    pub fn is_empty(&self) -> Result<bool, ReceiptAuditError> {
        self.require_complete()?;
        Ok(self.receipts.is_empty())
    }

    /// Bounded actual history for a reverse typed-operation join. This never
    /// falls back to current titles or treats unavailable replay as empty.
    pub fn receipts(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &InitialFilingAuditSubject>, ReceiptAuditError> {
        self.require_complete()?;
        Ok(self.receipts.values())
    }

    pub fn verify(&self, subject: &InitialFilingAuditSubject) -> Result<(), ReceiptAuditError> {
        self.require_complete()?;
        if self.receipts.get(&subject.commit) != Some(subject) {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(())
    }

    /// Used only while validating an already bounded, hash-checked checkpoint.
    pub(crate) fn checkpoint_complete(&self) -> bool {
        self.complete
            && !self.invalid
            && self.receipts.len() <= MAX_FILING_RECEIPTS
            && self
                .receipts
                .iter()
                .all(|(key, value)| *key == value.commit && value.valid())
    }

    /// Called by the real boot loader after the entire selected history/tail
    /// has been accepted. Empty history still needs this explicit transition.
    pub(crate) fn finish_replay(&mut self) -> Result<(), ReceiptAuditError> {
        if self.invalid || self.receipts.len() > MAX_FILING_RECEIPTS {
            return Err(ReceiptAuditError::Refused);
        }
        self.complete = true;
        self.verified_this_boot = true;
        Ok(())
    }

    pub(super) fn may_add(&self, commit: CommitToken) -> bool {
        self.require_complete().is_ok()
            && self.receipts.len() < MAX_FILING_RECEIPTS
            && !self.receipts.contains_key(&commit)
    }

    /// Observe only the actual immutable journal transaction. Malformed closed
    /// namespace entries poison readiness instead of disappearing from history.
    pub(crate) fn observe(&mut self, transaction: &EconomicTransaction) {
        if !matches!(
            transaction.idempotency.namespace.as_str(),
            FILING_NAMESPACE | house::NAMESPACE
        ) {
            return;
        }
        let Ok(subject) = subject_of(transaction) else {
            self.invalid = true;
            return;
        };
        if self.receipts.len() >= MAX_FILING_RECEIPTS || self.receipts.contains_key(&subject.commit)
        {
            self.invalid = true;
            return;
        }
        self.receipts.insert(subject.commit, subject);
    }
}

/// The bounded cause retains the two account identities even for a lawful
/// zero-fee product. The fee itself always comes from the actual journal lines.
pub(super) fn correlation(commit: CommitToken, payer: AccountId, sink: AccountId) -> String {
    format!("{commit}:{}:{}", payer.0, sink.0)
}

pub(super) fn house_correlation(
    commit: CommitToken,
    payer: AccountId,
    sink: AccountId,
    curator: ObjId,
) -> String {
    format!("{}:{}", correlation(commit, payer, sink), curator.0)
}

pub(super) fn subject_of(
    tx: &EconomicTransaction,
) -> Result<InitialFilingAuditSubject, ReceiptAuditError> {
    let refused = || ReceiptAuditError::Refused;
    let commit = CommitToken::from_hex(tx.idempotency.key.as_str()).map_err(|_| refused())?;
    let is_house = tx.idempotency.namespace.as_str() == house::NAMESPACE;
    let expected_namespace = if is_house {
        house::NAMESPACE
    } else {
        FILING_NAMESPACE
    };
    if tx.idempotency.namespace.as_str() != expected_namespace
        || tx.idempotency.key.as_str() != commit.to_hex()
        || tx.reverses.is_some()
        || !tx.hold_effects.is_empty()
        || !tx.custody_effects.is_empty()
        || tx.consumed_approval.is_some()
    {
        return Err(refused());
    }
    let EconomicCause::Ruleset {
        namespace,
        correlation: token,
    } = &tx.cause
    else {
        return Err(refused());
    };
    if namespace.as_str() != expected_namespace || token.len() > if is_house { 85 } else { 75 } {
        return Err(refused());
    }
    let mut parts = token.split(':');
    let (Some(raw_commit), Some(raw_payer), Some(raw_sink)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return Err(refused());
    };
    let payer = AccountId(raw_payer.parse::<u64>().map_err(|_| refused())?);
    let sink = AccountId(raw_sink.parse::<u64>().map_err(|_| refused())?);
    let curator = if is_house {
        let raw = parts.next().ok_or_else(refused)?;
        let curator = ObjId(raw.parse::<u32>().map_err(|_| refused())?);
        if raw != curator.0.to_string() || curator.0 == 0 {
            return Err(refused());
        }
        Some(curator)
    } else {
        None
    };
    let expected_correlation = match curator {
        Some(curator) => house_correlation(commit, payer, sink, curator),
        None => correlation(commit, payer, sink),
    };
    if parts.next().is_some()
        || raw_commit != commit.to_hex()
        || token.as_str() != expected_correlation
    {
        return Err(refused());
    }
    let PrincipalId::System(issuer) = &tx.actor else {
        return Err(refused());
    };
    let [effect] = tx.asset_effects.as_slice() else {
        return Err(refused());
    };
    if effect.kind != AssetEffectKind::Created
        || effect.object != effect.post.object
        || effect.post.last_journal_seq != tx.journal_seq
    {
        return Err(refused());
    }
    let fee = match tx.lines.as_slice() {
        [] => Amount::ZERO,
        [left, right] => {
            let (debit, credit) = if left.account == payer {
                (left, right)
            } else {
                (right, left)
            };
            if debit.account != payer
                || credit.account != sink
                || !debit.delta.is_negative()
                || credit.delta.is_negative()
                || credit.delta == Amount::ZERO
                || debit.instrument != credit.instrument
                || debit.delta.checked_neg() != Some(credit.delta)
            {
                return Err(refused());
            }
            credit.delta
        }
        _ => return Err(refused()),
    };
    if let Some(curator) = curator {
        let PrincipalId::Organization(organization) = &effect.post.title_owner else {
            return Err(refused());
        };
        let scope = effect.post.scope.ok_or_else(refused)?;
        let house =
            HouseFilingAttribution::new(curator, *organization, scope).map_err(|_| refused())?;
        return InitialFilingAuditSubject::new_house(
            commit,
            tx.id,
            tx.journal_seq,
            effect.object,
            effect.post.revision,
            payer,
            sink,
            fee,
            issuer.clone(),
            effect.post.catalog.clone().ok_or_else(refused)?,
            house,
        );
    }
    if effect.post.scope.is_some() {
        return Err(refused());
    }
    InitialFilingAuditSubject::new(
        commit,
        tx.id,
        tx.journal_seq,
        effect.object,
        effect.post.title_owner.clone(),
        effect.post.revision,
        payer,
        sink,
        fee,
        issuer.clone(),
        effect.post.catalog.clone().ok_or_else(refused)?,
    )
}
