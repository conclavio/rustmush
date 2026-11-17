//! **The governed-core resale receipt family** (doc 48 unit EX.2b): every resale derived from
//! its exact acknowledged journal entry, replay-verified at boot as the buy-back's are. The audit
//! is a projection of the journal; it adds no persisted row of its own (the checkpoint carries
//! the fold).
use super::*;
use serde::{Deserialize, Serialize};
pub(crate) const NAMESPACE: &str = "tacsim.core_resale.v1";
const MAX_RECEIPTS: usize = 65_536;

/// **One resale's receipt**: the commit, the journal entry, the buyer and the seller, the core
/// and the title revision read, the hull and her hold when she went with it, every account and
/// amount of the split, and the post-title images.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreResaleAuditSubject {
    pub commit: CommitToken,
    pub transaction: TransactionId,
    pub journal_seq: u64,
    pub posted_at_utc: i64,
    pub plan_hash: [u8; 32],
    pub buyer: ObjId,
    pub seller: ObjId,
    /// The resale's World IC second.
    pub at_ic: i64,
    pub core: ObjId,
    pub title_revision: u64,
    /// The hull the core was fitted in and sold with (Q-48-52's core leg).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull: Option<ObjId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull_revision: Option<u64>,
    /// Her hold's vault, re-homed to the buyer in the same union.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<VaultId>,
    pub buyer_account: AccountId,
    pub seller_account: AccountId,
    pub venue_account: AccountId,
    pub venue_permille: u16,
    pub levy_account: AccountId,
    pub levy_permille: u16,
    pub price: Amount,
    pub issuer: SystemPrincipalId,
    pub title: EconomicAsset,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull_title: Option<EconomicAsset>,
}
impl CoreResaleAuditSubject {
    /// The terms the split was made at.
    pub(super) fn terms(&self) -> Terms {
        Terms {
            venue: self.venue_account,
            venue_permille: self.venue_permille,
            levy: self.levy_account,
            levy_permille: self.levy_permille,
        }
    }
    pub fn validate(&self) -> Result<(), ReceiptAuditError> {
        let refused = || ReceiptAuditError::Refused;
        let ids = [
            Some(self.buyer),
            Some(self.seller),
            Some(self.core),
            self.hull,
        ];
        let distinct = ids
            .iter()
            .flatten()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == ids.iter().flatten().count();
        let accounts = [
            self.buyer_account,
            self.seller_account,
            self.venue_account,
            self.levy_account,
        ];
        if !distinct
            || ids.iter().flatten().any(|id| id.0 == 0)
            || accounts
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != accounts.len()
            || self.title_revision == 0
            || self.title_revision.checked_add(1) != Some(self.title.revision)
            || self.terms().split(self.price).is_none()
            || !self.bought(&self.title, self.core)
            || self.hull.is_some() != self.hull_revision.is_some()
            || self.hull.is_some() != self.hull_title.is_some()
            || (self.hold.is_some() && self.hull.is_none())
        {
            return Err(refused());
        }
        if let (Some(hull), Some(revision), Some(title)) =
            (self.hull, self.hull_revision, &self.hull_title)
        {
            if revision == 0 || revision.checked_add(1) != Some(title.revision) {
                return Err(refused());
            }
            if !self.bought(title, hull) {
                return Err(refused());
            }
        }
        let value = serde_json::to_value(self).map_err(|_| refused())?;
        mush_core::canonical_json_bytes_with_limit(&value, 16_384).map_err(|_| refused())?;
        Ok(())
    }
    /// A post-title image this entry wrote: the buyer's alone, live and unclaimed.
    fn bought(&self, title: &EconomicAsset, object: ObjId) -> bool {
        title.object == object
            && title.title_owner == PrincipalId::Object(self.buyer)
            && title.scope.is_none()
            && !title.frozen
            && title.frozen_reason.is_none()
            && title.stolen.is_none()
            && title.lifecycle == AssetLifecycle::Active
            && title.encumbrances.is_empty()
            && title.acquisition
                == Some(crate::record::AcquisitionRecord {
                    transaction: self.transaction,
                    at: self.posted_at_utc,
                })
            && title.last_journal_seq == self.journal_seq
    }
    /// The venue's cut this resale paid.
    #[must_use]
    pub fn venue_cut(&self) -> Amount {
        Amount::from_minor(self.terms().split(self.price).map_or(0, |(v, _, _)| v))
    }
    /// The levy this resale paid.
    #[must_use]
    pub fn levy(&self) -> Amount {
        Amount::from_minor(self.terms().split(self.price).map_or(0, |(_, l, _)| l))
    }
    /// What the seller was paid.
    #[must_use]
    pub fn proceeds(&self) -> Amount {
        Amount::from_minor(self.terms().split(self.price).map_or(0, |(_, _, n)| n))
    }
}

/// **The split's terms** a receipt carries: who took the venue's cut and the levy, and at what
/// rates. The config's, minus its action and owners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Terms {
    pub(super) venue: AccountId,
    pub(super) venue_permille: u16,
    pub(super) levy: AccountId,
    pub(super) levy_permille: u16,
}
impl Terms {
    pub(super) fn of(config: &CoreResaleConfig) -> Self {
        Self {
            venue: config.venue.account,
            venue_permille: config.venue_permille,
            levy: config.levy.account,
            levy_permille: config.levy_permille,
        }
    }
    /// As [`CoreResaleConfig::split`].
    fn split(self, price: Amount) -> Option<(i64, i64, i64)> {
        if u32::from(self.venue_permille) + u32::from(self.levy_permille) >= 1_000 {
            return None;
        }
        let price = price.minor();
        if price <= 0 {
            return None;
        }
        let part = |permille: u16| price.checked_mul(i64::from(permille)).map(|m| m / 1_000);
        let venue = part(self.venue_permille)?;
        let levy = part(self.levy_permille)?;
        let net = price.checked_sub(venue)?.checked_sub(levy)?;
        (net > 0).then_some((venue, levy, net))
    }
}

/// **The receipt audit**: every resale by commit. Boxed (one pointer in the fold), so the boot's
/// frame does not grow with it (the M7 memory-size bound on `load_boot_with_coverage`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CoreResaleReceiptAudit(Box<AuditState>);
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditState {
    receipts: BTreeMap<CommitToken, CoreResaleAuditSubject>,
    complete: bool,
    invalid: bool,
    #[serde(skip)]
    verified_this_boot: bool,
}
impl CoreResaleReceiptAudit {
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
    ) -> Result<impl ExactSizeIterator<Item = &CoreResaleAuditSubject>, ReceiptAuditError> {
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
    ) -> Result<Option<&CoreResaleAuditSubject>, ReceiptAuditError> {
        self.require_complete()?;
        Ok(self.0.receipts.get(&commit))
    }
    /// Whether `s` is the next legal receipt: a fresh commit and a well-formed subject.
    pub(crate) fn may_admit(&self, s: &CoreResaleAuditSubject) -> bool {
        self.require_complete().is_ok() && self.admissible(s)
    }
    fn admissible(&self, s: &CoreResaleAuditSubject) -> bool {
        self.0.receipts.len() < MAX_RECEIPTS
            && !self.0.receipts.contains_key(&s.commit)
            && s.validate().is_ok()
    }
    /// Install an admitted receipt. Infallible: [`Self::may_admit`] held at prepare and the
    /// bind's fence held since.
    pub(super) fn install(&mut self, s: CoreResaleAuditSubject) {
        self.0.receipts.insert(s.commit, s);
    }
    /// Fold one journal entry (boot replay and the live fold call this alike). An entry in the
    /// family's namespace that does not decode to an admissible resale marks the audit invalid.
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
    /// **The managed-title index's recovery**: every resale names a core the index knows as a
    /// managed core birth, and never a hull it knows.
    pub(crate) fn verify_titles(
        &self,
        index: &crate::managed_title::ManagedTitleIndex,
    ) -> Result<(), ReceiptAuditError> {
        self.require_complete()?;
        if self.0.receipts.values().any(|receipt| {
            index.core(receipt.core).is_none() || receipt.hull.is_some_and(|h| index.contains(h))
        }) {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(())
    }
}

/// The cause's correlation: every number the receipt keeps that the journal entry's lines and
/// effects do not already carry.
pub(super) fn correlation(
    (commit, buyer, at_ic): (CommitToken, ObjId, i64),
    config: &CoreResaleConfig,
    r: &RegistryCoreResale,
) -> String {
    correlation_of((commit, buyer, at_ic), Terms::of(config), r)
}
fn correlation_of(
    (commit, buyer, at_ic): (CommitToken, ObjId, i64),
    terms: Terms,
    r: &RegistryCoreResale,
) -> String {
    let (hull, hull_revision, hold) = r.hull.as_ref().map_or((0, 0, 0), |leg| {
        (leg.hull.0, leg.title_revision, leg.hold.map_or(0, |v| v.0))
    });
    format!(
        "resale:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
        commit,
        buyer.0,
        r.seller.0,
        r.core.0,
        r.title_revision,
        hull,
        hull_revision,
        hold,
        r.buyer_account.0,
        r.seller_account.0,
        terms.venue.0,
        terms.venue_permille,
        terms.levy.0,
        terms.levy_permille,
        r.price.minor(),
        at_ic
    )
}

pub(super) fn subject_of(
    tx: &EconomicTransaction,
) -> Result<CoreResaleAuditSubject, ReceiptAuditError> {
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
    if f.next() != Some("resale") || f.next() != Some(commit.to_hex().as_str()) {
        return Err(bad());
    }
    let number = |s: Option<&str>| -> Result<u64, ReceiptAuditError> {
        s.ok_or_else(bad)?.parse().map_err(|_| bad())
    };
    let object = |n: u64| u32::try_from(n).map(ObjId).map_err(|_| bad());
    let permille = |n: u64| u16::try_from(n).map_err(|_| bad());
    let buyer = object(number(f.next())?)?;
    let seller = object(number(f.next())?)?;
    let core = object(number(f.next())?)?;
    let title_revision = number(f.next())?;
    let hull = object(number(f.next())?)?;
    let hull_revision = number(f.next())?;
    let hold = number(f.next())?;
    let buyer_account = AccountId(number(f.next())?);
    let seller_account = AccountId(number(f.next())?);
    let terms = Terms {
        venue: AccountId(number(f.next())?),
        venue_permille: permille(number(f.next())?)?,
        levy: AccountId(number(f.next())?),
        levy_permille: permille(number(f.next())?)?,
    };
    let price: i64 = f.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
    let at_ic: i64 = f.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
    let leg = match (hull.0, hull_revision, hold) {
        (0, 0, 0) => None,
        (0, _, _) => return Err(bad()),
        (_, revision, hold) => Some(ResaleHull {
            hull,
            title_revision: revision,
            hold: (hold != 0).then_some(VaultId(hold)),
        }),
    };
    let request = RegistryCoreResale {
        core,
        title_revision,
        hull: leg,
        seller,
        seller_account,
        buyer_account,
        price: Amount::from_minor(price),
    };
    if f.next().is_some() || raw.as_str() != correlation_of((commit, buyer, at_ic), terms, &request)
    {
        return Err(bad());
    }
    let PrincipalId::System(issuer) = &tx.actor else {
        return Err(bad());
    };
    // Exactly the closed lines, each once (the journal keeps them in its own canonical order):
    // the buyer pays the price, the seller is paid the net, the venue and the sink their cuts.
    let (venue, levy, net) = terms.split(request.price).ok_or_else(bad)?;
    let mut expected = vec![(buyer_account, -price), (seller_account, net)];
    for (account, cut) in [(terms.venue, venue), (terms.levy, levy)] {
        if cut > 0 {
            expected.push((account, cut));
        }
    }
    if tx.lines.len() != expected.len()
        || expected.iter().any(|(account, delta)| {
            tx.lines
                .iter()
                .filter(|l| l.account == *account && l.delta.minor() == *delta)
                .count()
                != 1
        })
        || tx
            .lines
            .iter()
            .map(|l| l.account)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != tx.lines.len()
    {
        return Err(bad());
    }
    // Exactly one title transfer per object sold: the core, then the hull with it.
    let objects: Vec<ObjId> = std::iter::once(core)
        .chain(request.hull.as_ref().map(|leg| leg.hull))
        .collect();
    if tx.asset_effects.len() != objects.len()
        || tx
            .asset_effects
            .iter()
            .any(|e| e.kind != crate::AssetEffectKind::TitleTransferred)
    {
        return Err(bad());
    }
    let post = |object: ObjId| {
        let mut found = tx.asset_effects.iter().filter(|e| e.object == object);
        match (found.next(), found.next()) {
            (Some(effect), None) => Ok(effect.post.clone()),
            _ => Err(bad()),
        }
    };
    let title = post(core)?;
    let hull_title = match &request.hull {
        Some(leg) => Some(post(leg.hull)?),
        None => None,
    };
    let row = CoreResaleAuditSubject {
        commit,
        transaction: tx.id,
        journal_seq: tx.journal_seq,
        posted_at_utc: tx.posted_at,
        plan_hash: tx.plan_hash,
        buyer,
        seller,
        at_ic,
        core,
        title_revision,
        hull: request.hull.as_ref().map(|leg| leg.hull),
        hull_revision: request.hull.as_ref().map(|leg| leg.title_revision),
        hold: request.hull.as_ref().and_then(|leg| leg.hold),
        buyer_account,
        seller_account,
        venue_account: terms.venue,
        venue_permille: terms.venue_permille,
        levy_account: terms.levy,
        levy_permille: terms.levy_permille,
        price: request.price,
        issuer: issuer.clone(),
        title,
        hull_title,
    };
    row.validate()?;
    Ok(row)
}
