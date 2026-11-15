//! **The stored-core sale receipt family** (doc 47 CO.13): every sale derived from its exact
//! acknowledged journal entry, replay-verified at boot as the seizure's are. The audit is a
//! projection of the journal; it adds no persisted row of its own (the checkpoint carries the
//! fold, as it carries the seizure audit).
use super::*;
use mush_org::OrgId;
use serde::{Deserialize, Serialize};
pub(crate) const NAMESPACE: &str = "tacsim.core_sale.v1";
const MAX_RECEIPTS: usize = 65_536;

/// **One sale's receipt**: the commit, the journal entry, the seller, the core and the title
/// revision read, the vendor (its org, scope and account), the seller's payout account, the
/// price and the post-title image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreSaleAuditSubject {
    pub commit: CommitToken,
    pub transaction: TransactionId,
    pub journal_seq: u64,
    pub posted_at_utc: i64,
    pub plan_hash: [u8; 32],
    pub seller: ObjId,
    /// The sale's World IC second.
    pub at_ic: i64,
    pub core: ObjId,
    pub title_revision: u64,
    pub vendor: OrgId,
    pub vendor_scope: OrgScopeId,
    pub vendor_account: AccountId,
    pub seller_account: AccountId,
    pub price: Amount,
    pub issuer: SystemPrincipalId,
    pub title: EconomicAsset,
}
impl CoreSaleAuditSubject {
    pub fn validate(&self) -> Result<(), ReceiptAuditError> {
        if self.seller.0 == 0
            || self.core.0 == 0
            || self.seller == self.core
            || self.vendor_account == self.seller_account
            || self.title_revision == 0
            || self.title_revision.checked_add(1) != Some(self.title.revision)
            || self.price.minor() <= 0
            || self.title.object != self.core
            || self.title.title_owner != PrincipalId::Organization(self.vendor)
            || self.title.scope.is_some()
            || self.title.frozen
            || self.title.frozen_reason.is_some()
            || self.title.stolen.is_some()
            || self.title.lifecycle != AssetLifecycle::Active
            || !self.title.encumbrances.is_empty()
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

/// **The receipt audit**: every sale by commit. Boxed (one pointer in the fold), so the boot's
/// frame does not grow with it (the M7 memory-size bound on `load_boot_with_coverage`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CoreSaleReceiptAudit(Box<AuditState>);
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditState {
    receipts: BTreeMap<CommitToken, CoreSaleAuditSubject>,
    complete: bool,
    invalid: bool,
    #[serde(skip)]
    verified_this_boot: bool,
}
impl CoreSaleReceiptAudit {
    pub(crate) fn uninitialized(&self) -> bool {
        !self.0.complete && !self.0.invalid && self.0.receipts.is_empty()
    }
    pub fn require_complete(&self) -> Result<(), ReceiptAuditError> {
        if !self.0.complete || !self.0.verified_this_boot {
            return Err(ReceiptAuditError::Incomplete);
        }
        if self.0.invalid {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(())
    }
    pub(crate) fn require_absent(&self) -> Result<(), ReceiptAuditError> {
        self.require_complete()?;
        if !self.0.receipts.is_empty() {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(())
    }
    /// Every receipt, in commit order, once the audit is complete.
    ///
    /// # Errors
    /// The audit is not complete this boot, or is invalid.
    pub fn receipts(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &CoreSaleAuditSubject>, ReceiptAuditError> {
        self.require_complete()?;
        Ok(self.0.receipts.values())
    }
    /// The receipt of `commit`, once complete.
    ///
    /// # Errors
    /// As [`Self::receipts`].
    pub fn get(
        &self,
        commit: CommitToken,
    ) -> Result<Option<&CoreSaleAuditSubject>, ReceiptAuditError> {
        self.require_complete()?;
        Ok(self.0.receipts.get(&commit))
    }
    /// Whether `s` is the next legal receipt: a fresh commit and a well-formed subject.
    pub(crate) fn may_admit(&self, s: &CoreSaleAuditSubject) -> bool {
        self.require_complete().is_ok() && self.admissible(s)
    }
    fn admissible(&self, s: &CoreSaleAuditSubject) -> bool {
        self.0.receipts.len() < MAX_RECEIPTS
            && !self.0.receipts.contains_key(&s.commit)
            && s.validate().is_ok()
    }
    /// Install an admitted receipt. Infallible: [`Self::may_admit`] held at prepare and the
    /// bind's fence held since.
    pub(super) fn install(&mut self, s: CoreSaleAuditSubject) {
        self.0.receipts.insert(s.commit, s);
    }
    /// Fold one journal entry (boot replay and the live fold call this alike). An entry in the
    /// family's namespace that does not decode to an admissible sale marks the audit invalid.
    pub(crate) fn observe(&mut self, tx: &EconomicTransaction) {
        let ours = tx.idempotency.namespace == NAMESPACE
            || matches!(&tx.cause, EconomicCause::Ruleset { namespace, .. } if namespace == NAMESPACE);
        if !ours {
            return;
        }
        match subject_of(tx) {
            Ok(subject) if !self.0.invalid && self.admissible(&subject) => self.install(subject),
            _ => self.0.invalid = true,
        }
    }
    pub(crate) fn finish_replay(&mut self) -> Result<(), ReceiptAuditError> {
        if self.0.invalid || self.0.receipts.len() > MAX_RECEIPTS {
            return Err(ReceiptAuditError::Refused);
        }
        self.0.complete = true;
        self.0.verified_this_boot = true;
        Ok(())
    }
    pub(crate) fn checkpoint_complete(&self, rows: &[crate::checkpoint::IdemRow]) -> bool {
        // The checkpoint's complete key index proves an omitted inventory empty; a complete
        // flag alone proves neither direction (the charter transfer's rule).
        if self.uninitialized() {
            return !rows.iter().any(|r| r.ns.as_str() == NAMESPACE);
        }
        if !self.0.complete || self.0.invalid || self.0.receipts.len() > MAX_RECEIPTS {
            return false;
        }
        let mut expected = BTreeMap::new();
        for (key, receipt) in &self.0.receipts {
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
        expected.is_empty()
    }
    /// **The managed-title index's recovery**: every sale names a core the index knows as a
    /// managed core birth.
    pub(crate) fn verify_titles(
        &self,
        index: &crate::managed_title::ManagedTitleIndex,
    ) -> Result<(), ReceiptAuditError> {
        self.require_complete()?;
        if self
            .0
            .receipts
            .values()
            .any(|receipt| index.core(receipt.core).is_none())
        {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(())
    }
}

/// The cause's correlation: every number the receipt keeps that the journal entry's lines and
/// effects do not already carry.
pub(super) fn correlation(
    (commit, seller, at_ic): (CommitToken, ObjId, i64),
    vendor: &ScopedCoreAccount,
    r: &RegistryCoreSale,
) -> String {
    format!(
        "sale:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
        commit,
        seller.0,
        r.core.0,
        r.title_revision,
        vendor.organization.0,
        vendor.scope.0,
        vendor.account.0,
        r.seller_account.0,
        r.price.minor(),
        at_ic
    )
}

pub(super) fn subject_of(
    tx: &EconomicTransaction,
) -> Result<CoreSaleAuditSubject, ReceiptAuditError> {
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
    if f.next() != Some("sale") || f.next() != Some(commit.to_hex().as_str()) {
        return Err(bad());
    }
    let number = |s: Option<&str>| -> Result<u64, ReceiptAuditError> {
        s.ok_or_else(bad)?.parse().map_err(|_| bad())
    };
    let object = |n: u64| u32::try_from(n).map(ObjId).map_err(|_| bad());
    let seller = object(number(f.next())?)?;
    let core = object(number(f.next())?)?;
    let title_revision = number(f.next())?;
    let vendor = ScopedCoreAccount {
        organization: OrgId(number(f.next())?),
        scope: OrgScopeId(number(f.next())?),
        account: AccountId(number(f.next())?),
    };
    let seller_account = AccountId(number(f.next())?);
    let price: i64 = f.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
    let at_ic: i64 = f.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
    let request = RegistryCoreSale {
        core,
        title_revision,
        seller_account,
        price: Amount::from_minor(price),
    };
    if f.next().is_some() || raw.as_str() != correlation((commit, seller, at_ic), &vendor, &request)
    {
        return Err(bad());
    }
    let PrincipalId::System(issuer) = &tx.actor else {
        return Err(bad());
    };
    // Exactly the two closed lines (the journal keeps them in its own canonical order): the
    // vendor pays the price, the seller is paid it.
    let [a, b] = tx.lines.as_slice() else {
        return Err(bad());
    };
    let line = |account: AccountId, delta: Amount| {
        [a, b]
            .into_iter()
            .filter(|l| l.account == account && l.delta == delta)
            .count()
            == 1
    };
    if a.account == b.account
        || !line(vendor.account, Amount::from_minor(-price))
        || !line(seller_account, request.price)
    {
        return Err(bad());
    }
    let [effect] = tx.asset_effects.as_slice() else {
        return Err(bad());
    };
    if effect.object != core || effect.kind != crate::AssetEffectKind::TitleTransferred {
        return Err(bad());
    }
    let row = CoreSaleAuditSubject {
        commit,
        transaction: tx.id,
        journal_seq: tx.journal_seq,
        posted_at_utc: tx.posted_at,
        plan_hash: tx.plan_hash,
        seller,
        at_ic,
        core,
        title_revision,
        vendor: vendor.organization,
        vendor_scope: vendor.scope,
        vendor_account: vendor.account,
        seller_account,
        price: request.price,
        issuer: issuer.clone(),
        title: effect.post.clone(),
    };
    row.validate()?;
    Ok(row)
}
