//! Exact fee-only history, reconstructed from the real immutable E4 journal.
use super::*;
use serde::{Deserialize, Serialize};

pub(crate) const NAMESPACE: &str = "roll.line_update.v1";
const MAX_RECEIPTS: usize = 65_536;

/// Comparison data, never permission to post or a fabricated journal receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateAuditSubject {
    pub commit: CommitToken,
    pub transaction: TransactionId,
    pub journal_seq: u64,
    pub posted_at_utc: i64,
    pub plan_hash: [u8; 32],
    pub actor: ObjId,
    pub charter: ObjId,
    pub title_revision: u64,
    pub payer: AccountId,
    pub sink: AccountId,
    pub fee: Amount,
    pub instrument: crate::ids::InstrumentId,
    pub issuer: SystemPrincipalId,
}
impl UpdateAuditSubject {
    pub fn validate(&self) -> Result<(), ReceiptAuditError> {
        if self.actor.0 == 0
            || self.charter.0 == 0
            || self.title_revision == 0
            || self.payer == self.sink
            || self.fee.minor() <= 0
        {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateReceiptAudit {
    receipts: BTreeMap<CommitToken, UpdateAuditSubject>,
    complete: bool,
    invalid: bool,
    #[serde(skip)]
    verified_this_boot: bool,
}
impl UpdateReceiptAudit {
    pub(crate) fn uninitialized(&self) -> bool {
        !self.complete && !self.invalid && self.receipts.is_empty()
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
    pub fn verify(&self, expected: &UpdateAuditSubject) -> Result<(), ReceiptAuditError> {
        self.require_complete()?;
        expected.validate()?;
        if self.receipts.get(&expected.commit) != Some(expected) {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(())
    }
    pub fn receipts(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &UpdateAuditSubject>, ReceiptAuditError> {
        self.require_complete()?;
        Ok(self.receipts.values())
    }
    pub(crate) fn checkpoint_complete(&self, rows: &[crate::checkpoint::IdemRow]) -> bool {
        // The validated checkpoint contains the complete immutable key index.
        // That index can prove an old omitted inventory empty without replaying
        // unrelated money; a complete flag alone never proves either direction.
        if self.uninitialized() {
            return !rows.iter().any(|r| r.ns.as_str() == NAMESPACE);
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
        expected.is_empty()
    }
    pub(crate) fn finish_replay(&mut self) -> Result<(), ReceiptAuditError> {
        if self.invalid || self.receipts.len() > MAX_RECEIPTS {
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
    pub(super) fn install(&mut self, subject: UpdateAuditSubject) {
        self.receipts.insert(subject.commit, subject);
    }
    pub(crate) fn observe(&mut self, tx: &EconomicTransaction) {
        let cause_matches =
            matches!(&tx.cause, EconomicCause::Ruleset { namespace, .. } if namespace == NAMESPACE);
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
    request: &RegistryCompatibleUpdate,
    sink: AccountId,
) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}",
        commit, actor.0, request.charter.0, request.title_revision, request.payer.0, sink.0
    )
}

pub(super) fn subject_of(
    tx: &EconomicTransaction,
) -> Result<UpdateAuditSubject, ReceiptAuditError> {
    let bad = || ReceiptAuditError::Refused;
    if tx.idempotency.namespace != NAMESPACE
        || !tx.asset_effects.is_empty()
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
    if namespace != NAMESPACE || raw.len() > 128 {
        return Err(bad());
    }
    let mut fields = raw.split(':');
    let raw_commit = fields.next().ok_or_else(bad)?;
    let actor = ObjId(fields.next().ok_or_else(bad)?.parse().map_err(|_| bad())?);
    let charter = ObjId(fields.next().ok_or_else(bad)?.parse().map_err(|_| bad())?);
    let title_revision = fields.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
    let payer = AccountId(fields.next().ok_or_else(bad)?.parse().map_err(|_| bad())?);
    let sink = AccountId(fields.next().ok_or_else(bad)?.parse().map_err(|_| bad())?);
    let request = RegistryCompatibleUpdate {
        charter,
        title_revision,
        payer,
    };
    if fields.next().is_some()
        || raw_commit != commit.to_hex()
        || raw.as_str() != correlation(commit, actor, &request, sink)
    {
        return Err(bad());
    }
    let PrincipalId::System(issuer) = &tx.actor else {
        return Err(bad());
    };
    let [left, right] = tx.lines.as_slice() else {
        return Err(bad());
    };
    let (debit, credit) = if left.account == payer {
        (left, right)
    } else {
        (right, left)
    };
    if debit.account != payer
        || credit.account != sink
        || !debit.delta.is_negative()
        || credit.delta.minor() <= 0
        || debit.delta.checked_neg() != Some(credit.delta)
        || debit.instrument != credit.instrument
    {
        return Err(bad());
    }
    let subject = UpdateAuditSubject {
        commit,
        transaction: tx.id,
        journal_seq: tx.journal_seq,
        posted_at_utc: tx.posted_at,
        plan_hash: tx.plan_hash,
        actor,
        charter,
        title_revision,
        payer,
        sink,
        fee: credit.delta,
        instrument: credit.instrument.clone(),
        issuer: issuer.clone(),
    };
    subject.validate()?;
    Ok(subject)
}
