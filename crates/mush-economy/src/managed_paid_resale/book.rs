//! Bounded permanent native inventory. Decoding a checkpoint never supplies
//! health: journal/index completeness and every actual contract/hold must join.
use super::history::{self, PaidResaleHistoryV1};
use super::plans::{self, Proof};
use super::*;
use crate::contract::EconomicContract;
use crate::record::Hold;
use crate::record::{EconomicCause, EconomicTransaction, HoldPurpose};
use crate::{ContractId, HoldId};
use mush_core::CommitToken;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const MAX_HISTORIES: usize = 65_536;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PaidResaleBook {
    #[serde(deserialize_with = "rows")]
    histories: BTreeMap<CommitToken, PaidResaleHistoryV1>,
    complete: bool,
    invalid: bool,
    #[serde(skip)]
    verified: bool,
}
fn rows<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<CommitToken, PaidResaleHistoryV1>, D::Error> {
    struct Rows;
    impl<'de> serde::de::Visitor<'de> for Rows {
        type Value = BTreeMap<CommitToken, PaidResaleHistoryV1>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded distinct paid resale histories")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            if map.size_hint().is_some_and(|n| n > MAX_HISTORIES) {
                return Err(serde::de::Error::custom("resale history limit"));
            }
            let mut rows = BTreeMap::new();
            while let Some(key) = map.next_key::<CommitToken>()? {
                if rows.len() == MAX_HISTORIES || rows.contains_key(&key) {
                    return Err(serde::de::Error::custom(
                        "resale history duplicate or limit",
                    ));
                }
                let row = map.next_value::<PaidResaleHistoryV1>()?;
                row.validate().map_err(serde::de::Error::custom)?;
                if key != row.consent().operation {
                    return Err(serde::de::Error::custom("resale history key"));
                }
                rows.insert(key, row);
            }
            Ok(rows)
        }
    }
    d.deserialize_map(Rows)
}
impl PaidResaleBook {
    pub(crate) fn uninitialized(&self) -> bool {
        !self.complete && !self.invalid && self.histories.is_empty()
    }
    pub(crate) fn require_complete(&self) -> Result<(), ParticipantError> {
        if !self.complete || !self.verified || self.invalid {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn require_absent(&self) -> Result<(), ParticipantError> {
        self.require_complete()?;
        if !self.histories.is_empty() {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn get(&self, op: CommitToken) -> Result<&PaidResaleHistoryV1, ParticipantError> {
        self.find(op)?.ok_or_else(refused)
    }
    pub(super) fn find(
        &self,
        op: CommitToken,
    ) -> Result<Option<&PaidResaleHistoryV1>, ParticipantError> {
        self.require_complete()?;
        Ok(self.histories.get(&op))
    }
    pub(crate) fn histories(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &PaidResaleHistoryV1>, ParticipantError> {
        self.require_complete()?;
        Ok(self.histories.values())
    }
    fn validate(&self) -> Result<(), ParticipantError> {
        if self.invalid || self.histories.len() > MAX_HISTORIES {
            return Err(refused());
        }
        let mut contracts = BTreeSet::new();
        let mut holds = BTreeSet::new();
        let mut commits = BTreeSet::new();
        let mut offers = BTreeSet::new();
        let mut transactions = BTreeSet::new();
        let mut sequences = BTreeSet::new();
        for (key, row) in &self.histories {
            row.validate()?;
            if *key != row.consent().operation
                || !contracts.insert(row.contract_id())
                || !offers.insert(row.consent().terms.offer)
                || !commits.insert(*key)
                || !transactions.insert(row.acceptance().transaction())
                || !sequences.insert(row.acceptance().journal_seq())
            {
                return Err(refused());
            }
            if let Some(hold) = row.acceptance().hold()
                && !holds.insert(hold)
            {
                return Err(refused());
            }
            if let Some(t) = row.terminal()
                && (!commits.insert(t.decision().operation)
                    || !transactions.insert(t.transaction())
                    || !sequences.insert(t.journal_seq()))
            {
                return Err(refused());
            }
        }
        Ok(())
    }
    /// Called only after replay has consumed the actual complete checkpoint and
    /// suffix. A deserialized complete flag never substitutes for this join.
    pub(crate) fn finish_replay(
        &mut self,
        contracts_before: &BTreeMap<ContractId, EconomicContract>,
        holds_before: &BTreeMap<HoldId, Hold>,
        journal_holds: &BTreeMap<HoldId, Hold>,
    ) -> Result<(), ParticipantError> {
        self.verified = false;
        self.validate()?;
        let mut contracts = BTreeSet::<ContractId>::new();
        let mut holds = BTreeSet::new();
        for row in self.histories.values() {
            let contract = contracts_before
                .get(&row.contract_id())
                .ok_or_else(refused)?;
            let hold = row
                .acceptance()
                .hold()
                .map(|id| holds_before.get(&id).ok_or_else(refused))
                .transpose()?;
            row.verify_documents(contract, hold)?;
            contracts.insert(row.contract_id());
            if let Some(h) = hold {
                if journal_holds.get(&h.id) != Some(h) {
                    return Err(refused());
                }
                holds.insert(h.id);
            }
        }
        for c in contracts_before.values() {
            if c.consent
                .managed
                .as_ref()
                .and_then(|m| m.paid_resale_marker())
                .is_some()
                && !contracts.contains(&c.id)
            {
                return Err(refused());
            }
        }
        for h in holds_before.values().chain(journal_holds.values()) {
            let points_to_resale = matches!(h.purpose, HoldPurpose::Escrow { contract } if contracts.contains(&contract));
            if (h.holder.namespace.as_str() == NAMESPACE || points_to_resale)
                && (!holds.contains(&h.id) || journal_holds.get(&h.id) != Some(h))
            {
                return Err(refused());
            }
        }
        self.complete = true;
        self.verified = true;
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
            let a = row.acceptance();
            expected.insert(
                format!(
                    "{op}/{}",
                    if row.consent().mode == PaidResaleModeV1::Immediate {
                        "accept"
                    } else {
                        "fund"
                    }
                ),
                (
                    a.transaction(),
                    a.journal_seq(),
                    row.consent().accepted_at_utc,
                    a.plan_hash(),
                ),
            );
            if let Some(t) = row.terminal() {
                expected.insert(
                    format!(
                        "{op}/{}",
                        if t.decision().reason.settles() {
                            "settle"
                        } else {
                            "refund"
                        }
                    ),
                    (
                        t.transaction(),
                        t.journal_seq(),
                        t.decision().posted_at_utc,
                        t.plan_hash(),
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
    fn next(&self, tx: &EconomicTransaction) -> Result<PaidResaleHistoryV1, ParticipantError> {
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
        let row = match plans::decode(correlation.as_str())? {
            Proof::Accept { format: 1, intent } | Proof::Fund { format: 1, intent } => {
                if self.histories.len() == MAX_HISTORIES
                    || self.histories.contains_key(&intent.consent.operation)
                {
                    return Err(refused());
                }
                history::from_acceptance(tx, intent)?
            }
            Proof::Terminal {
                format: 1,
                acceptance,
                decision,
            } => history::with_terminal(
                self.histories.get(&acceptance).ok_or_else(refused)?,
                tx,
                decision,
            )?,
            _ => return Err(refused()),
        };
        // Exact one-use offer and disjoint native IDs are permanent, including
        // refunded attempts. This reads bounded history, never clones the book.
        for other in self
            .histories
            .values()
            .filter(|r| r.consent().operation != row.consent().operation)
        {
            if other.consent().terms.offer == row.consent().terms.offer
                || other.contract_id() == row.contract_id()
                || (row.acceptance().hold().is_some()
                    && other.acceptance().hold() == row.acceptance().hold())
            {
                return Err(refused());
            }
            let commits = [
                Some(other.consent().operation),
                other.terminal().map(|t| t.decision().operation),
            ];
            if commits.contains(&Some(row.consent().operation))
                || row
                    .terminal()
                    .is_some_and(|t| commits.contains(&Some(t.decision().operation)))
            {
                return Err(refused());
            }
            let old_tx = [
                Some(other.acceptance().transaction()),
                other.terminal().map(|t| t.transaction()),
            ];
            let old_seq = [
                Some(other.acceptance().journal_seq()),
                other.terminal().map(|t| t.journal_seq()),
            ];
            if old_tx.contains(&Some(tx.id)) || old_seq.contains(&Some(tx.journal_seq)) {
                return Err(refused());
            }
        }
        Ok(row)
    }
    pub(super) fn prepare(
        &self,
        tx: &EconomicTransaction,
    ) -> Result<PaidResaleHistoryV1, ParticipantError> {
        self.require_complete()?;
        self.next(tx)
    }
    pub(super) fn prepare_patch(
        &self,
        tx: &EconomicTransaction,
    ) -> Result<PreparedHistory, ParticipantError> {
        let after = self.prepare(tx)?;
        Ok(PreparedHistory {
            before: self.histories.get(&after.consent().operation).cloned(),
            count: self.histories.len(),
            after,
        })
    }
    fn install(&mut self, row: PaidResaleHistoryV1) {
        self.histories.insert(row.consent().operation, row);
    }
    pub(crate) fn observe(&mut self, tx: &EconomicTransaction) {
        let named = matches!(&tx.cause,EconomicCause::Ruleset{namespace,..} if namespace.as_str()==NAMESPACE);
        if tx.idempotency.namespace.as_str() != NAMESPACE && !named {
            return;
        }
        match self.next(tx) {
            Ok(row) => self.install(row),
            Err(_) => self.invalid = true,
        }
    }
}
pub(super) struct PreparedHistory {
    before: Option<PaidResaleHistoryV1>,
    count: usize,
    after: PaidResaleHistoryV1,
}
impl PreparedHistory {
    pub(super) fn row(&self) -> &PaidResaleHistoryV1 {
        &self.after
    }
    pub(super) fn matches(&self, state: &PaidResaleBook) -> bool {
        state.require_complete().is_ok()
            && state.histories.len() == self.count
            && state.histories.get(&self.after.consent().operation) == self.before.as_ref()
    }
    pub(super) fn apply(self, state: &mut PaidResaleBook) {
        state.install(self.after);
    }
}
