//! Bounded immutable purchase history, derived from the real E4 journal.
//! Current contracts/holds are projections, never substitutes for this proof.

use super::*;
use crate::record::{Hold, HoldEffectKind, HoldStatus};

const MAX_CORRELATION_BYTES: usize = 4096;
const MAX_HISTORY_BYTES: usize = 8192;
const TERMINAL_RESERVE_BYTES: usize = 2048;
// Funding needs two permanent Roll roots, inside its existing 65,536-record
// ceiling. Charge the complete future proof slot now; no terminal capacity is
// allocated later, and unrelated E4 history does not consume these slots.
const MAX_HISTORIES: usize = 32_768;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PayoutWire {
    account: AccountId,
    owner: PrincipalId,
    amount: crate::Amount,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PurchaseWire {
    quote: CommitToken,
    quote_digest: DefinitionDigest,
    product_digest: DefinitionDigest,
    buyer: ObjId,
    seller: ObjId,
    charter: ObjId,
    title_revision: u64,
    payer: AccountId,
    instrument: InstrumentId,
    price: crate::Amount,
    payouts: Vec<PayoutWire>,
    quoted_at_utc: i64,
    expires_at_utc: i64,
}
impl PurchaseWire {
    fn capture(p: &OriginalPurchase) -> Self {
        Self {
            quote: p.quote,
            quote_digest: p.quote_digest,
            product_digest: p.product_digest,
            buyer: p.buyer,
            seller: p.seller,
            charter: p.charter,
            title_revision: p.title_revision,
            payer: p.payer,
            instrument: p.instrument.clone(),
            price: p.price,
            payouts: p
                .payouts
                .iter()
                .map(|p| PayoutWire {
                    account: p.account,
                    owner: p.owner.clone(),
                    amount: p.amount,
                })
                .collect(),
            quoted_at_utc: p.quoted_at_utc,
            expires_at_utc: p.expires_at_utc,
        }
    }
    fn data(&self) -> Result<OriginalPurchase, PurchaseAuditError> {
        let p = OriginalPurchase {
            quote: self.quote,
            quote_digest: self.quote_digest,
            product_digest: self.product_digest,
            buyer: self.buyer,
            seller: self.seller,
            charter: self.charter,
            title_revision: self.title_revision,
            payer: self.payer,
            instrument: self.instrument.clone(),
            price: self.price,
            payouts: self
                .payouts
                .iter()
                .map(|p| PurchasePayout {
                    account: p.account,
                    owner: p.owner.clone(),
                    amount: p.amount,
                })
                .collect(),
            quoted_at_utc: self.quoted_at_utc,
            expires_at_utc: self.expires_at_utc,
        };
        p.validate_shape()
            .map_err(|_| PurchaseAuditError::Refused)?;
        Ok(p)
    }
}
// Untagged outer dispatch preserves the complete legacy nested object bytes.
// Both variants are strict and disjoint; new subjects require an explicit tag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum SubjectWire {
    Original(PurchaseWire),
    Entitlement(EntitlementSubjectWire),
    House(HouseSubjectWire),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum EntitlementSubjectWire {
    MindEntitlement {
        purchase: crate::managed_entitlement::subject::EntitlementWire,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum HouseSubjectWire {
    HouseMind {
        purchase: crate::managed_house_purchase::subject::HouseWire,
    },
}
impl SubjectWire {
    fn capture(p: &PurchaseSubject) -> Self {
        match p.recipe() {
            super::subject::Recipe::Original(p) => Self::Original(PurchaseWire::capture(p)),
            super::subject::Recipe::Entitlement(p) => {
                Self::Entitlement(EntitlementSubjectWire::MindEntitlement { purchase: p.into() })
            }
            super::subject::Recipe::House(p) => {
                Self::House(HouseSubjectWire::HouseMind { purchase: p.into() })
            }
        }
    }
    fn data(&self) -> Result<PurchaseSubject, PurchaseAuditError> {
        match self {
            Self::Original(p) => Ok(p.data()?.into()),
            Self::Entitlement(EntitlementSubjectWire::MindEntitlement { purchase }) => purchase
                .data()
                .and_then(PurchaseSubject::try_from)
                .map_err(|_| PurchaseAuditError::Refused),
            Self::House(HouseSubjectWire::HouseMind { purchase }) => purchase
                .data()
                .and_then(PurchaseSubject::try_from)
                .map_err(|_| PurchaseAuditError::Refused),
        }
    }
    fn namespace(&self) -> &'static str {
        match self {
            Self::Original(_) => NAMESPACE,
            Self::Entitlement(_) => crate::managed_entitlement::NAMESPACE,
            Self::House(_) => crate::managed_house_purchase::NAMESPACE,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Proof {
    Fund {
        format: u16,
        operation: CommitToken,
        effect: DefinitionRef,
        purchase: Box<SubjectWire>,
        at_ic: ClockStamp,
    },
    Grant {
        format: u16,
        operation: CommitToken,
        commit: CommitToken,
        issued_at_ic: ClockStamp,
    },
    Refund {
        format: u16,
        operation: CommitToken,
        commit: CommitToken,
    },
}
fn encode(proof: &Proof) -> Result<String, ParticipantError> {
    let value = serde_json::to_string(proof).map_err(|_| refused())?;
    if value.len() > MAX_CORRELATION_BYTES {
        return Err(refused());
    }
    Ok(value)
}
pub(super) fn funding_correlation(
    operation: CommitToken,
    effect: &DefinitionRef,
    purchase: &PurchaseSubject,
    at_ic: ClockStamp,
) -> Result<String, ParticipantError> {
    encode(&Proof::Fund {
        format: 1,
        operation,
        effect: effect.clone(),
        purchase: Box::new(SubjectWire::capture(purchase)),
        at_ic,
    })
}
pub(super) fn terminal_correlation(
    operation: CommitToken,
    commit: CommitToken,
    outcome: PurchaseOutcome,
    at_ic: ClockStamp,
) -> Result<String, ParticipantError> {
    encode(&match outcome {
        PurchaseOutcome::Granted => Proof::Grant {
            format: 1,
            operation,
            commit,
            issued_at_ic: at_ic,
        },
        PurchaseOutcome::Refunded => Proof::Refund {
            format: 1,
            operation,
            commit,
        },
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PurchaseAuditError {
    #[error("authoritative purchase replay is incomplete")]
    Incomplete,
    #[error("purchase evidence differs from authoritative history")]
    Refused,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PurchaseTerminalHistory {
    commit: CommitToken,
    granted: bool,
    transaction: TransactionId,
    journal_seq: u64,
    issued_at_ic: Option<ClockStamp>,
    hold: Hold,
    posted_at: i64,
    plan_hash: [u8; 32],
}
impl PurchaseTerminalHistory {
    pub fn commit(&self) -> CommitToken {
        self.commit
    }
    pub fn granted(&self) -> bool {
        self.granted
    }
    pub fn transaction(&self) -> TransactionId {
        self.transaction
    }
    pub fn journal_seq(&self) -> u64 {
        self.journal_seq
    }
    pub fn issued_at_ic(&self) -> Option<&ClockStamp> {
        self.issued_at_ic.as_ref()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PurchaseHistory {
    operation: CommitToken,
    effect: DefinitionRef,
    purchase: SubjectWire,
    funding: FundingReceipt,
    created_at: i64,
    hold: Hold,
    plan_hash: [u8; 32],
    terminal: Option<PurchaseTerminalHistory>,
}
impl PurchaseHistory {
    pub fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn effect(&self) -> &DefinitionRef {
        &self.effect
    }
    pub fn funding(&self) -> &FundingReceipt {
        &self.funding
    }
    pub fn terminal(&self) -> Option<&PurchaseTerminalHistory> {
        self.terminal.as_ref()
    }
    pub fn matches_purchase(&self, p: &OriginalPurchase, funding: &FundingReceipt) -> bool {
        self.purchase == SubjectWire::Original(PurchaseWire::capture(p)) && &self.funding == funding
    }
    pub fn matches_entitlement(
        &self,
        p: &crate::managed_entitlement::EntitlementPurchase,
        funding: &FundingReceipt,
    ) -> bool {
        PurchaseSubject::try_from(p.clone())
            .is_ok_and(|subject| self.matches_subject(&subject, funding))
    }
    /// Exact retained original recipe only. The common monetary projection of
    /// an entitlement recipe is never exposed as an original purchase.
    pub fn original(&self) -> Result<Option<OriginalPurchase>, PurchaseAuditError> {
        let subject = self.purchase.data()?;
        Ok(subject.original().cloned())
    }
    pub fn entitlement(
        &self,
    ) -> Result<Option<crate::managed_entitlement::EntitlementPurchase>, PurchaseAuditError> {
        Ok(self.purchase.data()?.entitlement().cloned())
    }
    pub fn house(
        &self,
    ) -> Result<Option<crate::managed_house_purchase::HousePurchase>, PurchaseAuditError> {
        Ok(self.purchase.data()?.house().cloned())
    }
    pub fn matches_house(
        &self,
        p: &crate::managed_house_purchase::HousePurchase,
        funding: &FundingReceipt,
    ) -> bool {
        PurchaseSubject::try_from(p.clone()).is_ok_and(|s| self.matches_subject(&s, funding))
    }
    pub(super) fn matches_subject(&self, p: &PurchaseSubject, funding: &FundingReceipt) -> bool {
        self.purchase == SubjectWire::capture(p) && &self.funding == funding
    }
    pub fn verify_documents(
        &self,
        contract: &EconomicContract,
        hold: &Hold,
    ) -> Result<(), PurchaseAuditError> {
        let subject = self.purchase.data()?;
        let marker = subject
            .marker(self.operation, self.effect.clone())
            .map_err(|_| PurchaseAuditError::Refused)?;
        let mut expected = super::contract(
            &subject,
            marker,
            self.funding.contract,
            self.funding.hold,
            self.created_at,
        )
        .map_err(|_| PurchaseAuditError::Refused)?;
        let expected_hold = if let Some(t) = &self.terminal {
            expected.state = if t.granted {
                ContractState::Settled {
                    transaction: t.transaction,
                }
            } else {
                ContractState::Cancelled
            };
            expected.revision = 2;
            &t.hold
        } else {
            &self.hold
        };
        if contract != &expected
            || hold != expected_hold
            || expected.consent_hash != self.funding.consent_hash
        {
            return Err(PurchaseAuditError::Refused);
        }
        Ok(())
    }
    fn validate(&self) -> Result<(), PurchaseAuditError> {
        let fail = || PurchaseAuditError::Refused;
        let subject = self.purchase.data()?;
        let p = subject.common();
        let namespace = subject.namespace();
        if !self.bounded()
            || !subject.matches_action(&self.effect, "fund")
            || self.created_at < p.quoted_at_utc
            || self.created_at >= p.expires_at_utc
            || !matches!(self.funding.at_ic.clock,ClockRef::WorldIc{generation} if generation!=0)
        {
            return Err(fail());
        }
        let h = &self.hold;
        if h.id != self.funding.hold
            || h.account != p.payer
            || h.amount != p.price
            || h.placed_by != p.buyer.principal()
            || h.holder
                != IdempotencyKey::new(namespace, format!("{}/fund", self.operation.to_hex()))
            || h.purpose
                != (HoldPurpose::Escrow {
                    contract: self.funding.contract,
                })
            || h.beneficiary != Some(crate::api::contract_principal())
            || h.expires.is_some()
            || h.status != HoldStatus::Held
            || h.revision != 1
            || h.last_journal_seq != self.funding.journal_seq
        {
            return Err(fail());
        }
        let marker = subject
            .marker(self.operation, self.effect.clone())
            .map_err(|_| fail())?;
        let contract = super::contract(
            &subject,
            marker,
            self.funding.contract,
            self.funding.hold,
            self.created_at,
        )
        .map_err(|_| fail())?;
        if contract.consent_hash != self.funding.consent_hash
            || crate::hash::plan_hash(&self.funding_plan()?) != self.plan_hash
        {
            return Err(fail());
        }
        if let Some(t) = &self.terminal {
            if t.commit == self.operation
                || t.transaction == self.funding.transaction
                || t.journal_seq <= self.funding.journal_seq
                || t.posted_at < self.created_at
                || t.granted != t.issued_at_ic.is_some()
                || t.issued_at_ic.as_ref().is_some_and(|stamp| {
                    stamp.clock != self.funding.at_ic.clock || stamp.at < self.funding.at_ic.at
                })
                || serde_json::to_vec(t).map_err(|_| fail())?.len() > TERMINAL_RESERVE_BYTES
            {
                return Err(fail());
            }
            let mut expected = h.clone();
            expected.status = if t.granted {
                HoldStatus::Consumed {
                    transaction: t.transaction,
                    amount: p.price,
                }
            } else {
                HoldStatus::Released
            };
            expected.revision = 2;
            expected.last_journal_seq = t.journal_seq;
            if t.hold != expected || crate::hash::plan_hash(&self.terminal_plan(t)?) != t.plan_hash
            {
                return Err(fail());
            }
        }
        Ok(())
    }
    // Independent closed-recipe reconstruction binds retained proof data to
    // the actual plan hashes in immutable idempotency parts. These are proof
    // checks only: live authorization still runs existing E4 builders/engine.
    fn funding_plan(&self) -> Result<EconomyPlan, PurchaseAuditError> {
        let subject = self.purchase.data()?;
        let p = subject.common();
        let namespace = subject.namespace();
        let cause = encode(&Proof::Fund {
            format: 1,
            operation: self.operation,
            effect: self.effect.clone(),
            purchase: Box::new(self.purchase.clone()),
            at_ic: self.funding.at_ic.clone(),
        })
        .map_err(|_| PurchaseAuditError::Refused)?;
        let mut plan = EconomyPlan::new(
            IdempotencyKey::new(namespace, format!("{}/fund", self.operation.to_hex())),
            p.buyer.principal(),
            EconomicCause::Ruleset {
                namespace: namespace.into(),
                correlation: cause.into(),
            },
            Vec::new(),
        );
        plan.description = subject.funding_description().into();
        plan.visibility = EconomicVisibility::Parties;
        plan.hold_ops.push(HoldOp::Place {
            account: p.payer,
            amount: p.price,
            beneficiary: Some(crate::api::contract_principal()),
            purpose: HoldPurpose::Escrow {
                contract: self.funding.contract,
            },
            expires: None,
        });
        Ok(plan)
    }
    fn terminal_plan(
        &self,
        t: &PurchaseTerminalHistory,
    ) -> Result<EconomyPlan, PurchaseAuditError> {
        let fail = || PurchaseAuditError::Refused;
        let subject = self.purchase.data()?;
        let p = subject.common();
        let namespace = subject.namespace();
        let proof = match &t.issued_at_ic {
            Some(stamp) => Proof::Grant {
                format: 1,
                operation: self.operation,
                commit: t.commit,
                issued_at_ic: stamp.clone(),
            },
            None => Proof::Refund {
                format: 1,
                operation: self.operation,
                commit: t.commit,
            },
        };
        let transition = if t.granted { "grant" } else { "refund" };
        let mut lines = Vec::new();
        if t.granted {
            lines.push(crate::plan::RequestedLine {
                account: p.payer,
                delta: p.price.checked_neg().ok_or_else(fail)?,
                memo: format!("contract {} escrow", self.funding.contract).into(),
            });
            for payout in &p.payouts {
                lines.push(crate::plan::RequestedLine {
                    account: payout.account,
                    delta: payout.amount,
                    memo: format!("contract {} payout", self.funding.contract).into(),
                });
            }
        }
        let mut plan = EconomyPlan::new(
            IdempotencyKey::new(
                namespace,
                format!("{}/{transition}", self.operation.to_hex()),
            ),
            p.buyer.principal(),
            EconomicCause::Ruleset {
                namespace: namespace.into(),
                correlation: encode(&proof).map_err(|_| fail())?.into(),
            },
            lines,
        );
        plan.description = format!(
            "{} of contract {}",
            if t.granted { "settlement" } else { "refund" },
            self.funding.contract
        );
        plan.visibility = EconomicVisibility::Parties;
        plan.hold_ops.push(if t.granted {
            HoldOp::Capture {
                hold: self.funding.hold,
                amount: p.price,
            }
        } else {
            HoldOp::Release {
                hold: self.funding.hold,
            }
        });
        Ok(plan)
    }
    fn bounded(&self) -> bool {
        let limit = if self.terminal.is_none() {
            MAX_HISTORY_BYTES - TERMINAL_RESERVE_BYTES
        } else {
            MAX_HISTORY_BYTES
        };
        serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() <= limit)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PurchaseReceiptAudit {
    histories: BTreeMap<CommitToken, PurchaseHistory>,
    complete: bool,
    invalid: bool,
    #[serde(skip)]
    verified_this_boot: bool,
}
impl PurchaseReceiptAudit {
    pub(crate) fn uninitialized(&self) -> bool {
        !self.complete && !self.invalid && self.histories.is_empty()
    }
    pub fn require_complete(&self) -> Result<(), PurchaseAuditError> {
        if !self.complete || !self.verified_this_boot {
            return Err(PurchaseAuditError::Incomplete);
        }
        if self.invalid {
            return Err(PurchaseAuditError::Refused);
        }
        Ok(())
    }
    pub fn receipts(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &PurchaseHistory>, PurchaseAuditError> {
        self.require_complete()?;
        Ok(self.histories.values())
    }
    pub fn get(&self, operation: CommitToken) -> Result<&PurchaseHistory, PurchaseAuditError> {
        self.require_complete()?;
        self.histories
            .get(&operation)
            .ok_or(PurchaseAuditError::Refused)
    }
    pub(crate) fn checkpoint_complete(&self, rows: &[crate::checkpoint::IdemRow]) -> bool {
        if !self.complete || !self.valid_shape() {
            return false;
        }
        // Every closed-namespace key in immutable checkpoint parts must have an
        // exact proof, and every proof must have its original key. A retained
        // complete flag or an empty map alone proves neither direction.
        let mut expected = BTreeMap::new();
        for history in self.histories.values() {
            expected.insert(
                (
                    history.purchase.namespace().to_owned(),
                    format!("{}/fund", history.operation.to_hex()),
                ),
                (
                    history.funding.transaction,
                    history.funding.journal_seq,
                    history.created_at,
                    history.plan_hash,
                ),
            );
            if let Some(t) = &history.terminal {
                expected.insert(
                    (
                        history.purchase.namespace().to_owned(),
                        format!(
                            "{}/{}",
                            history.operation.to_hex(),
                            if t.granted { "grant" } else { "refund" }
                        ),
                    ),
                    (t.transaction, t.journal_seq, t.posted_at, t.plan_hash),
                );
            }
        }
        for row in rows.iter().filter(|row| closed_namespace(row.ns.as_str())) {
            let Some((tx, seq, at, hash)) =
                expected.remove(&(row.ns.to_string(), row.key.to_string()))
            else {
                return false;
            };
            let encoded: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
            if row.tx != tx || row.seq != seq || row.posted_at != at || row.plan_hash != encoded {
                return false;
            }
        }
        expected.is_empty()
    }
    fn valid_shape(&self) -> bool {
        !self.invalid
            && self.histories.len() <= MAX_HISTORIES
            && self
                .histories
                .iter()
                .all(|(key, row)| *key == row.operation && row.validate().is_ok())
    }
    pub(crate) fn finish_replay(&mut self) -> Result<(), PurchaseAuditError> {
        if !self.valid_shape() {
            return Err(PurchaseAuditError::Refused);
        }
        self.complete = true;
        self.verified_this_boot = true;
        Ok(())
    }
    pub(crate) fn observe(&mut self, tx: &EconomicTransaction) {
        if !closed_namespace(tx.idempotency.namespace.as_str())
            && !matches!(&tx.cause,EconomicCause::Ruleset{namespace,..} if closed_namespace(namespace.as_str()))
        {
            return;
        }
        match self.next(tx) {
            Ok(row) => {
                self.histories.insert(row.operation, row);
            }
            Err(_) => self.invalid = true,
        }
    }
    pub(super) fn prepare(
        &self,
        tx: &EconomicTransaction,
    ) -> Result<Box<PreparedHistory>, ParticipantError> {
        self.require_complete().map_err(|_| refused())?;
        let after = self.next(tx).map_err(|_| refused())?;
        Ok(Box::new(PreparedHistory {
            before: self.histories.get(&after.operation).cloned(),
            after,
            count: self.histories.len(),
            complete: self.complete,
            invalid: self.invalid,
            verified: self.verified_this_boot,
        }))
    }
    fn next(&self, tx: &EconomicTransaction) -> Result<PurchaseHistory, PurchaseAuditError> {
        let fail = || PurchaseAuditError::Refused;
        if self.invalid
            || !closed_namespace(tx.idempotency.namespace.as_str())
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
        if namespace.as_str() != tx.idempotency.namespace.as_str()
            || correlation.len() > MAX_CORRELATION_BYTES
        {
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
                effect: definition,
                purchase,
                at_ic,
            } => {
                if self.histories.len() >= MAX_HISTORIES
                    || self.histories.contains_key(&operation)
                    || !matches!(at_ic.clock,ClockRef::WorldIc{generation} if generation!=0)
                    || tx.idempotency.key.as_str() != format!("{}/fund", operation.to_hex())
                    || !tx.lines.is_empty()
                    || effect.kind != HoldEffectKind::Placed
                {
                    return Err(fail());
                }
                let subject = purchase.data()?;
                let p = subject.common();
                if !subject.matches_action(&definition, "fund")
                    || subject.namespace() != namespace.as_str()
                {
                    return Err(fail());
                }
                let HoldPurpose::Escrow { contract: id } = &effect.hold.purpose else {
                    return Err(fail());
                };
                let h = &effect.hold;
                if tx.actor != p.buyer.principal()
                    || tx.posted_at < p.quoted_at_utc
                    || tx.posted_at >= p.expires_at_utc
                    || h.account != p.payer
                    || h.amount != p.price
                    || h.placed_by != tx.actor
                    || h.holder != tx.idempotency
                    || h.beneficiary != Some(crate::api::contract_principal())
                    || h.expires.is_some()
                    || h.status != HoldStatus::Held
                    || h.revision != 1
                    || h.last_journal_seq != tx.journal_seq
                {
                    return Err(fail());
                }
                let marker = subject
                    .marker(operation, definition.clone())
                    .map_err(|_| fail())?;
                let c = super::contract(&subject, marker, *id, h.id, tx.posted_at)
                    .map_err(|_| fail())?;
                PurchaseHistory {
                    operation,
                    effect: definition,
                    purchase: *purchase,
                    created_at: tx.posted_at,
                    hold: h.clone(),
                    plan_hash: tx.plan_hash,
                    terminal: None,
                    funding: FundingReceipt {
                        contract: *id,
                        hold: h.id,
                        transaction: tx.id,
                        journal_seq: tx.journal_seq,
                        consent_hash: c.consent_hash,
                        at_ic,
                    },
                }
            }
            Proof::Grant {
                format: 1,
                operation,
                commit,
                issued_at_ic,
            } => self.terminal_row(tx, operation, commit, Some(issued_at_ic))?,
            Proof::Refund {
                format: 1,
                operation,
                commit,
            } => self.terminal_row(tx, operation, commit, None)?,
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
        issued: Option<ClockStamp>,
    ) -> Result<PurchaseHistory, PurchaseAuditError> {
        let fail = || PurchaseAuditError::Refused;
        let mut row = self.histories.get(&operation).cloned().ok_or_else(fail)?;
        if row.terminal.is_some()
            || commit == operation
            || tx.id == row.funding.transaction
            || tx.journal_seq <= row.funding.journal_seq
            || tx.posted_at < row.created_at
        {
            return Err(fail());
        }
        let subject = row.purchase.data()?;
        let p = subject.common();
        if tx.idempotency.namespace.as_str() != subject.namespace() {
            return Err(fail());
        }
        let grant = issued.is_some();
        let transition = if grant { "grant" } else { "refund" };
        if tx.actor != p.buyer.principal()
            || tx.idempotency.key.as_str() != format!("{}/{transition}", operation.to_hex())
        {
            return Err(fail());
        }
        if let Some(stamp) = &issued
            && (stamp.clock != row.funding.at_ic.clock || stamp.at < row.funding.at_ic.at)
        {
            return Err(fail());
        }
        let [effect] = tx.hold_effects.as_slice() else {
            return Err(fail());
        };
        let mut hold = row.hold.clone();
        hold.status = if grant {
            HoldStatus::Consumed {
                transaction: tx.id,
                amount: p.price,
            }
        } else {
            HoldStatus::Released
        };
        hold.revision = 2;
        hold.last_journal_seq = tx.journal_seq;
        if effect.kind
            != if grant {
                HoldEffectKind::Captured
            } else {
                HoldEffectKind::Released
            }
            || effect.hold != hold
        {
            return Err(fail());
        }
        // Retain the gross debit separately from a royalty credit to the payer.
        // Netting here would let a smaller fabricated debit masquerade as capture.
        let mut lines = Vec::new();
        for line in &tx.lines {
            if line.instrument != p.instrument {
                return Err(fail());
            }
            lines.push((line.account, line.delta.minor()));
        }
        let mut expected = Vec::new();
        if grant {
            expected.push((p.payer, p.price.checked_neg().ok_or_else(fail)?.minor()));
            expected.extend(p.payouts.iter().map(|p| (p.account, p.amount.minor())));
        }
        lines.sort();
        expected.sort();
        if lines != expected {
            return Err(fail());
        }
        let terminal = PurchaseTerminalHistory {
            commit,
            granted: grant,
            transaction: tx.id,
            journal_seq: tx.journal_seq,
            issued_at_ic: issued,
            hold,
            posted_at: tx.posted_at,
            plan_hash: tx.plan_hash,
        };
        if serde_json::to_vec(&terminal).map_err(|_| fail())?.len() > TERMINAL_RESERVE_BYTES {
            return Err(fail());
        }
        row.terminal = Some(terminal);
        Ok(row)
    }
}
pub(super) struct PreparedHistory {
    before: Option<PurchaseHistory>,
    after: PurchaseHistory,
    count: usize,
    complete: bool,
    invalid: bool,
    verified: bool,
}
impl PreparedHistory {
    pub(super) fn matches(&self, a: &PurchaseReceiptAudit) -> bool {
        a.histories.len() == self.count
            && a.histories.get(&self.after.operation) == self.before.as_ref()
            && a.complete == self.complete
            && a.invalid == self.invalid
            && a.verified_this_boot == self.verified
    }
    pub(super) fn apply(self: Box<Self>, a: &mut PurchaseReceiptAudit) {
        a.histories.insert(self.after.operation, self.after);
    }
}

#[cfg(test)]
mod capacity_tests;

#[cfg(test)]
mod entitlement_tests;
