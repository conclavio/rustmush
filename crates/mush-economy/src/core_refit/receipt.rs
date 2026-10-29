//! Permanent native journal evidence for the closed fitting reservation.
//! No current title, decoded receipt or arbitrary plan opens this replay latch.
use super::plans::{self, FundingIntent, Proof};
use super::{HISTORY_BYTES, MAX_ORDERS, NAMESPACE, RefitError as Error};
use crate::plan::EconomyPlan;
use crate::record::{
    EconomicCause, EconomicTransaction, Hold, HoldEffectKind, HoldPurpose, HoldStatus,
};
use crate::{EconomySnapshot, HoldId, TransactionId};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefitFundingReceipt {
    pub(crate) commit: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) transaction: TransactionId,
    pub(crate) journal_seq: u64,
    pub(crate) posted_at: i64,
    pub(crate) plan_hash: [u8; 32],
    pub(crate) hold: Hold,
}
impl RefitFundingReceipt {
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn transaction(&self) -> TransactionId {
        self.transaction
    }
    pub const fn journal_seq(&self) -> u64 {
        self.journal_seq
    }
    pub const fn hold(&self) -> HoldId {
        self.hold.id
    }
    pub const fn posted_at_utc(&self) -> i64 {
        self.posted_at
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum RefitTerminalResult {
    Installed { replacement: DefinitionDigest },
    Cancelled {},
}
impl RefitTerminalResult {
    pub const fn replacement(&self) -> Option<DefinitionDigest> {
        match self {
            Self::Installed { replacement } => Some(*replacement),
            Self::Cancelled {} => None,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefitTerminalReceipt {
    pub(crate) commit: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) transaction: TransactionId,
    pub(crate) journal_seq: u64,
    pub(crate) posted_at: i64,
    pub(crate) at: ClockStamp,
    pub(crate) plan_hash: [u8; 32],
    pub(crate) hold: Hold,
    pub(crate) result: RefitTerminalResult,
}
impl RefitTerminalReceipt {
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn transaction(&self) -> TransactionId {
        self.transaction
    }
    pub const fn journal_seq(&self) -> u64 {
        self.journal_seq
    }
    pub const fn posted_at_utc(&self) -> i64 {
        self.posted_at
    }
    pub fn at_ic(&self) -> &ClockStamp {
        &self.at
    }
    pub fn result(&self) -> &RefitTerminalResult {
        &self.result
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefitPaymentHistory {
    pub(super) intent: Box<FundingIntent>,
    pub(crate) funding: Box<RefitFundingReceipt>,
    #[serde(deserialize_with = "required_option")]
    pub(crate) terminal: Option<Box<RefitTerminalReceipt>>,
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
impl RefitPaymentHistory {
    pub fn subject(&self) -> &super::RefitSubject {
        &self.intent.subject
    }
    pub fn policy(&self) -> &super::RefitPaymentPolicy {
        &self.intent.policy
    }
    pub fn funding(&self) -> &RefitFundingReceipt {
        &self.funding
    }
    pub fn terminal(&self) -> Option<&RefitTerminalReceipt> {
        self.terminal.as_deref()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > HISTORY_BYTES {
            return Err(Error::Capacity);
        }
        let row: Self = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
        row.validate()?;
        let exact = mush_core::mechanics::canonical_json_bytes_with_limit(
            &serde_json::to_value(&row).map_err(|_| Error::Invalid)?,
            HISTORY_BYTES,
        )
        .map_err(|_| Error::Capacity)?;
        if exact != bytes {
            return Err(Error::Invalid);
        }
        Ok(row)
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.intent.validate()?;
        let f = &self.funding;
        let s = &self.intent.subject;
        let plan = plans::funding(&self.intent, f.commit, &f.action)?;
        if crate::hash::plan_hash(&plan) != f.plan_hash
            || f.posted_at < self.intent.supplier.observed_utc
            || f.posted_at >= self.intent.supplier.expires_utc
            || f.hold.account != s.payer
            || f.hold.amount != self.intent.price()?
            || f.hold.placed_by != plan.actor
            || f.hold.holder != plan.idempotency
            || f.hold.beneficiary != Some(PrincipalId::System(self.intent.policy.issuer.clone()))
            || f.hold.purpose
                != (HoldPurpose::Reservation {
                    kind: super::HOLD_PURPOSE.into(),
                    id: s.order,
                })
            || f.hold.expires.is_some()
            || f.hold.status != HoldStatus::Held
            || f.hold.revision != 1
            || f.hold.last_journal_seq != f.journal_seq
        {
            return Err(Error::Integrity);
        }
        if let Some(t) = &self.terminal {
            let install = matches!(t.result, RefitTerminalResult::Installed { .. });
            let plan = plans::terminal(
                &self.intent,
                f.commit,
                f.hold.id,
                t.commit,
                &t.action,
                &t.at,
                t.result.replacement(),
            )?;
            if t.transaction == f.transaction
                || t.journal_seq <= f.journal_seq
                || t.posted_at < f.posted_at
                || crate::hash::plan_hash(&plan) != t.plan_hash
            {
                return Err(Error::Integrity);
            }
            let mut expected = f.hold.clone();
            expected.status = if install {
                HoldStatus::Consumed {
                    transaction: t.transaction,
                    amount: self.intent.price()?,
                }
            } else {
                HoldStatus::Released
            };
            expected.revision = 2;
            expected.last_journal_seq = t.journal_seq;
            if t.hold != expected {
                return Err(Error::Integrity);
            }
        }
        bounded(self, HISTORY_BYTES)?;
        Ok(())
    }
    pub fn verify_hold(&self, snapshot: &EconomySnapshot) -> Result<(), Error> {
        self.validate()?;
        let expected = self
            .terminal
            .as_ref()
            .map_or(&self.funding.hold, |t| &t.hold);
        if snapshot.hold(expected.id) != Some(expected) {
            return Err(Error::Integrity);
        }
        Ok(())
    }
    pub(crate) fn require_terminal_room(&self) -> Result<(), Error> {
        if self.terminal.is_some() {
            return Err(Error::Integrity);
        }
        // Measure the actual complete receipt shape at maximum scalar widths.
        // This temporary data is only a size probe, never replay or authority.
        let mut row = self.clone();
        let p = &self.intent.policy;
        let mut hold = self.funding.hold.clone();
        hold.revision = u64::MAX;
        hold.last_journal_seq = u64::MAX;
        for complete in [false, true] {
            hold.status = if complete {
                HoldStatus::Consumed {
                    transaction: TransactionId(u64::MAX),
                    amount: self.intent.price()?,
                }
            } else {
                HoldStatus::Released
            };
            let result = if complete {
                RefitTerminalResult::Installed {
                    replacement: DefinitionDigest::from_bytes([255; 32]),
                }
            } else {
                RefitTerminalResult::Cancelled {}
            };
            row.terminal = Some(Box::new(RefitTerminalReceipt {
                commit: CommitToken::from_bytes([255; 16]),
                action: p.action_ref(if complete {
                    super::INSTALL_ACTION
                } else {
                    super::CANCEL_ACTION
                })?,
                transaction: TransactionId(u64::MAX),
                journal_seq: u64::MAX,
                posted_at: i64::MIN,
                at: ClockStamp {
                    clock: mush_core::ClockRef::WorldIc {
                        generation: u64::MAX,
                    },
                    at: i64::MIN,
                },
                plan_hash: [255; 32],
                hold: hold.clone(),
                result,
            }));
            bounded(&row, HISTORY_BYTES)?;
        }
        Ok(())
    }
}
fn bounded<T: Serialize>(row: &T, max: usize) -> Result<(), Error> {
    mush_core::mechanics::canonical_json_bytes_with_limit(
        &serde_json::to_value(row).map_err(|_| Error::Invalid)?,
        max,
    )
    .map_err(|_| Error::Capacity)?;
    Ok(())
}
fn require_transaction(
    tx: &EconomicTransaction,
    plan: &EconomyPlan,
    instrument: &crate::InstrumentId,
) -> Result<(), Error> {
    if tx.idempotency != plan.idempotency
        || tx.cause != plan.cause
        || tx.actor != plan.actor
        || tx.description != plan.description
        || tx.visibility != plan.visibility
        || tx.plan_hash != crate::hash::plan_hash(plan)
        || tx.reverses.is_some()
        || tx.consumed_approval.is_some()
        || !tx.custody_effects.is_empty()
        || !tx.asset_effects.is_empty()
    {
        return Err(Error::Integrity);
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
    if actual != expected || tx.lines.iter().any(|l| &l.instrument != instrument) {
        return Err(Error::Integrity);
    }
    Ok(())
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RefitReceiptAudit {
    #[serde(deserialize_with = "rows")]
    histories: BTreeMap<CommitToken, RefitPaymentHistory>,
    complete: bool,
    invalid: bool,
    #[serde(skip)]
    verified: bool,
}
fn rows<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<CommitToken, RefitPaymentHistory>, D::Error> {
    struct Rows;
    impl<'de> serde::de::Visitor<'de> for Rows {
        type Value = BTreeMap<CommitToken, RefitPaymentHistory>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded distinct core refit histories")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            if map.size_hint().is_some_and(|n| n > MAX_ORDERS) {
                return Err(serde::de::Error::custom("core history limit"));
            }
            let mut rows = BTreeMap::new();
            while let Some(key) = map.next_key::<CommitToken>()? {
                if rows.len() == MAX_ORDERS || rows.contains_key(&key) {
                    return Err(serde::de::Error::custom("core history duplicate or limit"));
                }
                let row = map.next_value::<RefitPaymentHistory>()?;
                row.validate().map_err(serde::de::Error::custom)?;
                if key != row.funding.commit {
                    return Err(serde::de::Error::custom("core history key"));
                }
                rows.insert(key, row);
            }
            Ok(rows)
        }
    }
    d.deserialize_map(Rows)
}
impl RefitReceiptAudit {
    pub(crate) fn uninitialized(&self) -> bool {
        !self.complete && !self.invalid && self.histories.is_empty()
    }
    pub(crate) fn require_complete(&self) -> Result<(), Error> {
        if !self.complete || !self.verified {
            return Err(Error::Incomplete);
        }
        if self.invalid {
            return Err(Error::Integrity);
        }
        Ok(())
    }
    pub(crate) fn get(&self, acceptance: CommitToken) -> Result<&RefitPaymentHistory, Error> {
        self.require_complete()?;
        self.histories.get(&acceptance).ok_or(Error::Integrity)
    }
    pub(crate) fn histories(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &RefitPaymentHistory>, Error> {
        self.require_complete()?;
        Ok(self.histories.values())
    }
    pub(crate) fn require_absent(&self) -> Result<(), Error> {
        self.require_complete()?;
        if !self.histories.is_empty() {
            return Err(Error::Integrity);
        }
        Ok(())
    }
    pub(crate) fn finish_replay(&mut self) -> Result<(), Error> {
        self.valid_shape()?;
        self.complete = true;
        self.verified = true;
        Ok(())
    }
    fn valid_shape(&self) -> Result<(), Error> {
        if self.invalid || self.histories.len() > MAX_ORDERS {
            return Err(Error::Integrity);
        }
        let mut orders = BTreeSet::new();
        let mut holds = BTreeSet::new();
        let mut offers = BTreeSet::new();
        let mut installed = BTreeSet::new();
        for (key, h) in &self.histories {
            h.validate()?;
            if *key != h.funding.commit
                || !orders.insert(h.intent.subject.order)
                || !holds.insert(h.funding.hold.id)
                || !offers.insert(h.intent.subject.offer)
                || (h
                    .terminal
                    .as_ref()
                    .is_some_and(|t| matches!(t.result, RefitTerminalResult::Installed { .. }))
                    && !installed.insert(h.intent.subject.incoming))
            {
                return Err(Error::Integrity);
            }
        }
        Ok(())
    }
    pub(crate) fn checkpoint_complete(&self, rows: &[crate::checkpoint::IdemRow]) -> bool {
        // An old checkpoint may omit this family only when its actual complete
        // immutable idempotency inventory proves that no Core operation exists.
        if self.uninitialized() {
            return rows.iter().all(|r| r.ns != NAMESPACE);
        }
        if !self.complete || self.valid_shape().is_err() {
            return false;
        }
        let mut expected = BTreeMap::new();
        for h in self.histories.values() {
            let f = &h.funding;
            expected.insert(
                format!("{}/accept", f.commit.to_hex()),
                (f.transaction, f.journal_seq, f.posted_at, f.plan_hash),
            );
            if let Some(t) = &h.terminal {
                expected.insert(
                    format!(
                        "{}/{}",
                        f.commit.to_hex(),
                        if matches!(t.result, RefitTerminalResult::Installed { .. }) {
                            "install"
                        } else {
                            "cancel"
                        }
                    ),
                    (t.transaction, t.journal_seq, t.posted_at, t.plan_hash),
                );
            }
        }
        for r in rows.iter().filter(|r| r.ns == NAMESPACE) {
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
        if !touches(tx) {
            return;
        }
        match self.next(tx) {
            Ok(row) => {
                self.histories.insert(row.funding.commit, row);
            }
            Err(_) => self.invalid = true,
        }
    }
    pub(super) fn prepare(&self, tx: &EconomicTransaction) -> Result<PreparedHistory, Error> {
        self.require_complete()?;
        let after = self.next(tx)?;
        let before = self.histories.get(&after.funding.commit).cloned();
        Ok(PreparedHistory {
            before,
            after,
            count: self.histories.len(),
        })
    }
    fn next(&self, tx: &EconomicTransaction) -> Result<RefitPaymentHistory, Error> {
        if self.invalid || tx.idempotency.namespace != NAMESPACE {
            return Err(Error::Integrity);
        }
        let EconomicCause::Ruleset {
            namespace,
            correlation,
        } = &tx.cause
        else {
            return Err(Error::Integrity);
        };
        if namespace != NAMESPACE {
            return Err(Error::Integrity);
        }
        let proof = plans::decode_correlation(correlation)?;
        let [effect] = tx.hold_effects.as_slice() else {
            return Err(Error::Integrity);
        };
        let row = match proof {
            Proof::Accept {
                format: 1,
                commit,
                action,
                intent,
            } => {
                if self.histories.len() >= MAX_ORDERS
                    || self.histories.contains_key(&commit)
                    || self.histories.values().any(|h| {
                        h.intent.subject.order == intent.subject.order
                            || h.intent.subject.offer == intent.subject.offer
                            || h.funding.hold.id == effect.hold.id
                            || (h.intent.subject.incoming == intent.subject.incoming
                                && !h.terminal.as_ref().is_some_and(|t| {
                                    matches!(t.result, RefitTerminalResult::Cancelled {})
                                }))
                            || (h.intent.subject.hull == intent.subject.hull
                                && h.terminal.is_none())
                    })
                    || effect.kind != HoldEffectKind::Placed
                    || !tx.asset_effects.is_empty()
                {
                    return Err(Error::Integrity);
                }
                let plan = plans::funding(&intent, commit, &action)?;
                require_transaction(tx, &plan, &intent.policy.instrument)?;
                let row = RefitPaymentHistory {
                    intent,
                    funding: Box::new(RefitFundingReceipt {
                        commit,
                        action,
                        transaction: tx.id,
                        journal_seq: tx.journal_seq,
                        posted_at: tx.posted_at,
                        plan_hash: tx.plan_hash,
                        hold: effect.hold.clone(),
                    }),
                    terminal: None,
                };
                row.require_terminal_room()?;
                row
            }
            Proof::Install {
                format: 1,
                acceptance,
                commit,
                action,
                at,
                replacement,
            } => self.terminal_row(tx, acceptance, commit, action, at, Some(replacement))?,
            Proof::Cancel {
                format: 1,
                acceptance,
                commit,
                action,
                at,
            } => self.terminal_row(tx, acceptance, commit, action, at, None)?,
            _ => return Err(Error::Integrity),
        };
        row.validate()?;
        Ok(row)
    }
    fn terminal_row(
        &self,
        tx: &EconomicTransaction,
        acceptance: CommitToken,
        commit: CommitToken,
        action: DefinitionRef,
        at: ClockStamp,
        replacement: Option<DefinitionDigest>,
    ) -> Result<RefitPaymentHistory, Error> {
        let install = replacement.is_some();
        let mut row = self
            .histories
            .get(&acceptance)
            .cloned()
            .ok_or(Error::Integrity)?;
        if row.terminal.is_some() {
            return Err(Error::Integrity);
        }
        let plan = plans::terminal(
            &row.intent,
            acceptance,
            row.funding.hold.id,
            commit,
            &action,
            &at,
            replacement,
        )?;
        require_transaction(tx, &plan, &row.intent.policy.instrument)?;
        let [effect] = tx.hold_effects.as_slice() else {
            return Err(Error::Integrity);
        };
        let result = if install {
            if effect.kind != HoldEffectKind::Captured {
                return Err(Error::Integrity);
            }
            RefitTerminalResult::Installed {
                replacement: replacement.ok_or(Error::Integrity)?,
            }
        } else {
            if effect.kind != HoldEffectKind::Released {
                return Err(Error::Integrity);
            }
            RefitTerminalResult::Cancelled {}
        };
        row.terminal = Some(Box::new(RefitTerminalReceipt {
            commit,
            action,
            transaction: tx.id,
            journal_seq: tx.journal_seq,
            posted_at: tx.posted_at,
            at,
            plan_hash: tx.plan_hash,
            hold: effect.hold.clone(),
            result,
        }));
        Ok(row)
    }
}
pub(crate) fn touches(tx: &EconomicTransaction) -> bool {
    tx.idempotency.namespace==NAMESPACE
        ||matches!(&tx.cause,EconomicCause::Ruleset{namespace,..}if namespace==NAMESPACE)
        ||tx.hold_effects.iter().any(|e|matches!(&e.hold.purpose,HoldPurpose::Reservation{kind,..}if kind==super::HOLD_PURPOSE))
}
pub(super) struct PreparedHistory {
    before: Option<RefitPaymentHistory>,
    after: RefitPaymentHistory,
    count: usize,
}
impl PreparedHistory {
    pub fn row(&self) -> &RefitPaymentHistory {
        &self.after
    }
    pub fn matches(&self, audit: &RefitReceiptAudit) -> bool {
        audit.require_complete().is_ok()
            && audit.histories.len() == self.count
            && audit.histories.get(&self.after.funding.commit) == self.before.as_ref()
    }
    pub fn apply(self, audit: &mut RefitReceiptAudit) {
        audit
            .histories
            .insert(self.after.funding.commit, self.after);
    }
}
