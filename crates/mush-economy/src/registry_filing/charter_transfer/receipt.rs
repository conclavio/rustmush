//! Native charter-sale history, derived from exact acknowledged journal entries.
use super::*;
use serde::{Deserialize, Serialize};
pub(crate) const NAMESPACE: &str = "roll.charter_transfer.v1";
const MAX_RECEIPTS: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharterTransferAuditSubject {
    pub commit: CommitToken,
    pub transaction: TransactionId,
    pub journal_seq: u64,
    pub posted_at_utc: i64,
    pub plan_hash: [u8; 32],
    pub offer: CommitToken,
    pub offer_digest: DefinitionDigest,
    pub seller: ObjId,
    pub buyer: ObjId,
    pub charter: ObjId,
    pub title_revision: u64,
    pub payer: AccountId,
    pub seller_account: AccountId,
    pub price: Amount,
    pub instrument: crate::ids::InstrumentId,
    pub issuer: SystemPrincipalId,
    pub title: EconomicAsset,
}
impl CharterTransferAuditSubject {
    pub fn validate(&self) -> Result<(), ReceiptAuditError> {
        if self.seller.0 == 0
            || self.buyer.0 == 0
            || self.charter.0 == 0
            || self.seller == self.buyer
            || self.payer == self.seller_account
            || self.title_revision == 0
            || self.title_revision.checked_add(1) != Some(self.title.revision)
            || self.price.minor() <= 0
            || self.offer == self.commit
            || self.title.object != self.charter
            || self.title.title_owner != PrincipalId::Object(self.buyer)
            || self.title.scope.is_some()
            || self.title.frozen
            || self.title.frozen_reason.is_some()
            || self.title.stolen.is_some()
            || self.title.lifecycle != AssetLifecycle::Active
            || !self.title.encumbrances.is_empty()
            || self.title.catalog.is_none()
            || self.title.assignment.is_some()
            || self.title.maintenance_account.is_some()
            || self.title.acquisition
                != Some(crate::record::AcquisitionRecord {
                    transaction: self.transaction,
                    at: self.posted_at_utc,
                })
            || self.title.last_journal_seq != self.journal_seq
        {
            return Err(ReceiptAuditError::Refused);
        }
        let value = serde_json::to_value(self).map_err(|_| ReceiptAuditError::Refused)?;
        mush_core::canonical_json_bytes_with_limit(&value, 16_384)
            .map_err(|_| ReceiptAuditError::Refused)?;
        Ok(())
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharterTransferReceiptAudit {
    receipts: BTreeMap<CommitToken, CharterTransferAuditSubject>,
    // Rebuilt from exact births plus every later journal effect, including
    // generic namespaces. Omission cannot prove absence after a filing birth.
    #[serde(default)]
    lineages: BTreeMap<ObjId, super::lineage::Lineage>,
    complete: bool,
    invalid: bool,
    #[serde(skip)]
    verified_this_boot: bool,
}
impl CharterTransferReceiptAudit {
    pub(crate) fn uninitialized(&self) -> bool {
        !self.complete && !self.invalid && self.receipts.is_empty() && self.lineages.is_empty()
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
    pub fn verify(&self, expected: &CharterTransferAuditSubject) -> Result<(), ReceiptAuditError> {
        self.require_complete()?;
        expected.validate()?;
        if self.receipts.get(&expected.commit) != Some(expected) {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(())
    }
    pub fn receipts(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &CharterTransferAuditSubject>, ReceiptAuditError>
    {
        self.require_complete()?;
        Ok(self.receipts.values())
    }
    pub(crate) fn checkpoint_complete(&self, rows: &[crate::checkpoint::IdemRow]) -> bool {
        // The validated checkpoint contains the complete immutable key index.
        // That index can prove an old omitted inventory empty without replaying
        // unrelated money; a complete flag alone never proves either direction.
        if self.uninitialized() {
            return !rows.iter().any(|r| {
                r.ns.as_str() == NAMESPACE || super::lineage::is_birth_namespace(r.ns.as_str())
            });
        }
        if !self.complete || self.invalid || self.receipts.len() > MAX_RECEIPTS {
            return false;
        }
        let mut expected = BTreeMap::new();
        for (key, receipt) in &self.receipts {
            if *key != receipt.commit || receipt.validate().is_err() {
                return false;
            }
            expected.insert(key.to_hex(), receipt);
        }
        for row in rows.iter().filter(|r| r.ns.as_str() == NAMESPACE) {
            let Some(receipt) = expected.remove(row.key.as_str()) else {
                return false;
            };
            let hash: String = receipt
                .plan_hash
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            if row.tx != receipt.transaction
                || row.seq != receipt.journal_seq
                || row.posted_at != receipt.posted_at_utc
                || row.plan_hash != hash
            {
                return false;
            }
        }
        if !expected.is_empty() || self.lineages.len() > MAX_RECEIPTS {
            return false;
        }
        let mut births = BTreeMap::new();
        for (object, history) in &self.lineages {
            if history.check().is_err()
                || *object != history.object()
                || births
                    .insert(history.birth().commit().to_hex(), history.birth())
                    .is_some()
            {
                return false;
            }
        }
        for row in rows
            .iter()
            .filter(|r| super::lineage::is_birth_namespace(r.ns.as_str()))
        {
            let Some(birth) = births.remove(row.key.as_str()) else {
                return false;
            };
            if !super::lineage::birth_key_matches(birth, row) {
                return false;
            }
        }
        births.is_empty() && self.sales_match()
    }
    pub(crate) fn finish_replay(&mut self) -> Result<(), ReceiptAuditError> {
        if self.invalid
            || self.receipts.len() > MAX_RECEIPTS
            || self.lineages.len() > MAX_RECEIPTS
            || !self.sales_match()
        {
            return Err(ReceiptAuditError::Refused);
        }
        self.complete = true;
        self.verified_this_boot = true;
        Ok(())
    }
    pub(crate) fn require_absent(&self) -> Result<(), ReceiptAuditError> {
        self.require_complete()?;
        if !self.receipts.is_empty() {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(())
    }
    pub(super) fn may_add(&self, commit: CommitToken) -> bool {
        self.require_complete().is_ok()
            && self.receipts.len() < MAX_RECEIPTS
            && !self.receipts.contains_key(&commit)
    }
    pub(super) fn install(&mut self, subject: CharterTransferAuditSubject) {
        self.receipts.insert(subject.commit, subject);
    }
    fn sales_match(&self) -> bool {
        let mut counts = BTreeMap::<ObjId, u64>::new();
        for sale in self.receipts.values() {
            if !self.lineages.contains_key(&sale.charter) {
                return false;
            }
            *counts.entry(sale.charter).or_default() += 1;
        }
        self.lineages.values().all(|l| {
            l.check().is_ok() && counts.get(&l.object()).copied().unwrap_or(0) == l.sales()
        })
    }
    pub(crate) fn verify_lineages(
        &self,
        filings: &ReceiptAudit,
        assets: &BTreeMap<ObjId, EconomicAsset>,
    ) -> Result<(), ReceiptAuditError> {
        self.require_complete()?;
        let births = filings.receipts()?;
        if births.len() != self.lineages.len() {
            return Err(ReceiptAuditError::Refused);
        }
        for birth in births {
            let (object, _) = birth.managed_title_birth();
            let history = self
                .lineages
                .get(&object)
                .ok_or(ReceiptAuditError::Refused)?;
            if history.birth() != birth || assets.get(&object) != Some(history.title()) {
                return Err(ReceiptAuditError::Refused);
            }
        }
        Ok(())
    }
    pub(crate) fn prepare_birth(
        &self,
        tx: &EconomicTransaction,
    ) -> Result<super::lineage::Lineage, ReceiptAuditError> {
        self.require_complete()?;
        let row = super::lineage::Lineage::born(tx)?;
        if !self.birth_available(row.object()) {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(row)
    }
    pub(crate) fn birth_available(&self, object: ObjId) -> bool {
        self.require_complete().is_ok()
            && self.lineages.len() < MAX_RECEIPTS
            && !self.lineages.contains_key(&object)
    }
    pub(crate) fn install_lineage(&mut self, row: super::lineage::Lineage) {
        self.lineages.insert(row.object(), row);
    }
    pub(crate) fn lineage(&self, object: ObjId) -> Option<&super::lineage::Lineage> {
        self.lineages.get(&object)
    }
    pub(super) fn prepare_sale_lineage(
        &self,
        tx: &EconomicTransaction,
        sale: &CharterTransferAuditSubject,
    ) -> Result<super::lineage::Lineage, ReceiptAuditError> {
        self.require_complete()?;
        let [effect] = tx.asset_effects.as_slice() else {
            return Err(ReceiptAuditError::Refused);
        };
        self.lineages
            .get(&sale.charter)
            .ok_or(ReceiptAuditError::Refused)?
            .advance(tx, effect, Some(sale))
    }
    pub(crate) fn prepare_pending_lineage(
        &self,
        tx: &EconomicTransaction,
        history: &crate::managed_charter_transfer::CharterPendingHistory,
    ) -> Result<super::lineage::Lineage, ReceiptAuditError> {
        self.require_complete()?;
        let [effect] = tx.asset_effects.as_slice() else {
            return Err(ReceiptAuditError::Refused);
        };
        self.lineages
            .get(&history.consent().terms.charter)
            .ok_or(ReceiptAuditError::Refused)?
            .advance_pending(tx, effect, history)
    }
    pub(crate) fn verify_pending_lineages(
        &self,
        pending: &crate::managed_charter_transfer::history::CharterPendingAudit,
    ) -> Result<(), ReceiptAuditError> {
        self.require_complete()?;
        let mut counts = BTreeMap::<ObjId, u64>::new();
        for history in pending
            .histories()
            .map_err(|_| ReceiptAuditError::Refused)?
        {
            let object = history.consent().terms.charter;
            let lineage = self
                .lineages
                .get(&object)
                .ok_or(ReceiptAuditError::Refused)?;
            if lineage.birth().commit() != history.consent().terms.birth {
                return Err(ReceiptAuditError::Refused);
            }
            if history.terminal().is_some_and(|t| t.title().is_some()) {
                let n = counts.entry(object).or_default();
                *n = n.checked_add(1).ok_or(ReceiptAuditError::Refused)?;
            }
        }
        if self
            .lineages
            .values()
            .any(|l| l.pending_sales() != counts.get(&l.object()).copied().unwrap_or(0))
        {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(())
    }
    pub(crate) fn observe(&mut self, tx: &EconomicTransaction) {
        // Old direct callers cannot authorize the new namespace. Actual fold
        // uses the separately replayed pending inventory below.
        self.observe_with_pending(tx, &Default::default())
    }
    pub(crate) fn observe_with_pending(
        &mut self,
        tx: &EconomicTransaction,
        pending: &crate::managed_charter_transfer::history::CharterPendingAudit,
    ) {
        let cause_matches =
            matches!(&tx.cause, EconomicCause::Ruleset { namespace, .. } if namespace == NAMESPACE);
        if super::lineage::is_birth_namespace(tx.idempotency.namespace.as_str()) {
            match super::lineage::Lineage::born(tx) {
                Ok(row)
                    if self.lineages.len() < MAX_RECEIPTS
                        && !self.lineages.contains_key(&row.object()) =>
                {
                    self.install_lineage(row);
                }
                _ => self.invalid = true,
            }
            return;
        }
        let sale = if tx.idempotency.namespace == NAMESPACE || cause_matches {
            match subject_of(tx) {
                Ok(row) => Some(row),
                Err(_) => {
                    self.invalid = true;
                    return;
                }
            }
        } else {
            None
        };
        for effect in &tx.asset_effects {
            if let Some(before) = self.lineages.get(&effect.object) {
                let next = if tx.idempotency.namespace.as_str()
                    == crate::managed_charter_transfer::NAMESPACE
                {
                    pending
                        .pending_settlement(tx)
                        .map_err(|_| ReceiptAuditError::Refused)
                        .and_then(|h| before.advance_pending(tx, effect, h))
                } else {
                    before.advance(tx, effect, sale.as_ref())
                };
                match next {
                    Ok(after) => {
                        self.install_lineage(after);
                    }
                    Err(_) => {
                        self.invalid = true;
                        return;
                    }
                }
            } else if sale.is_some() {
                self.invalid = true;
                return;
            }
        }
        if tx.idempotency.namespace != NAMESPACE && !cause_matches {
            return;
        }
        let Ok(subject) = subject_of(tx) else {
            self.invalid = true;
            return;
        };
        if self.receipts.len() >= MAX_RECEIPTS || self.receipts.contains_key(&subject.commit) {
            self.invalid = true;
            return;
        }
        self.receipts.insert(subject.commit, subject);
    }
}

pub(super) fn correlation(
    commit: CommitToken,
    actor: ObjId,
    r: &RegistryCharterTransfer,
) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
        commit,
        actor.0,
        r.offer,
        r.offer_digest,
        r.seller.0,
        r.charter.0,
        r.title_revision,
        r.payer.0,
        r.seller_account.0,
        r.price.minor()
    )
}
pub(super) fn subject_of(
    tx: &EconomicTransaction,
) -> Result<CharterTransferAuditSubject, ReceiptAuditError> {
    let bad = || ReceiptAuditError::Refused;
    if tx.idempotency.namespace != NAMESPACE
        || !tx.hold_effects.is_empty()
        || !tx.custody_effects.is_empty()
        || tx.consumed_approval.is_some()
        || tx.reverses.is_some()
    {
        return Err(bad());
    }
    let commit = CommitToken::from_hex(tx.idempotency.key.as_str()).map_err(|_| bad())?;
    if tx.idempotency.key.as_str() != commit.to_hex() {
        return Err(bad());
    }
    let EconomicCause::Ruleset {
        namespace,
        correlation: raw,
    } = &tx.cause
    else {
        return Err(bad());
    };
    if namespace != NAMESPACE || raw.len() > 512 {
        return Err(bad());
    }
    let mut f = raw.split(':');
    let raw_commit = f.next().ok_or_else(bad)?;
    let buyer = ObjId(f.next().ok_or_else(bad)?.parse().map_err(|_| bad())?);
    let offer = CommitToken::from_hex(f.next().ok_or_else(bad)?).map_err(|_| bad())?;
    let offer_digest = DefinitionDigest::from_hex(f.next().ok_or_else(bad)?).map_err(|_| bad())?;
    let seller = ObjId(f.next().ok_or_else(bad)?.parse().map_err(|_| bad())?);
    let charter = ObjId(f.next().ok_or_else(bad)?.parse().map_err(|_| bad())?);
    let title_revision = f.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
    let payer = AccountId(f.next().ok_or_else(bad)?.parse().map_err(|_| bad())?);
    let seller_account = AccountId(f.next().ok_or_else(bad)?.parse().map_err(|_| bad())?);
    let price = Amount::from_minor(f.next().ok_or_else(bad)?.parse().map_err(|_| bad())?);
    let request = RegistryCharterTransfer {
        offer,
        offer_digest,
        charter,
        title_revision,
        seller,
        buyer,
        payer,
        seller_account,
        price,
    };
    if f.next().is_some()
        || raw_commit != commit.to_hex()
        || raw.as_str() != correlation(commit, buyer, &request)
    {
        return Err(bad());
    }
    let PrincipalId::System(issuer) = &tx.actor else {
        return Err(bad());
    };
    let [effect] = tx.asset_effects.as_slice() else {
        return Err(bad());
    };
    if effect.object != charter || effect.kind != crate::AssetEffectKind::TitleTransferred {
        return Err(bad());
    }
    let [a, b] = tx.lines.as_slice() else {
        return Err(bad());
    };
    let (debit, credit) = if a.account == payer { (a, b) } else { (b, a) };
    if debit.account != payer
        || credit.account != seller_account
        || debit.delta.checked_neg() != Some(price)
        || credit.delta != price
        || debit.instrument != credit.instrument
    {
        return Err(bad());
    }
    let row = CharterTransferAuditSubject {
        commit,
        transaction: tx.id,
        journal_seq: tx.journal_seq,
        posted_at_utc: tx.posted_at,
        plan_hash: tx.plan_hash,
        offer,
        offer_digest,
        seller,
        buyer,
        charter,
        title_revision,
        payer,
        seller_account,
        price,
        instrument: debit.instrument.clone(),
        issuer: issuer.clone(),
        title: effect.post.clone(),
    };
    row.validate()?;
    Ok(row)
}
