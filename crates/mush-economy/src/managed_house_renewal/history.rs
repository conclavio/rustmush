//! Nonmonetary owner evidence. This is not journal-derived funding proof.

use super::{
    CLOSE_ACTION, DRAFT_ACTION, RenewalAttemptV1, RenewalOffer, RenewalOutcome,
    UnfundedCloseReason, action_matches, contract, refused,
};
use crate::record::HoldPurpose;
use crate::{ContractId, EconomySnapshot};
use mush_core::participant::ParticipantError;
use mush_core::{ClockStamp, CommitToken, DefinitionRef};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const MAX_RECORDS: usize = 32_768;
pub(super) const MAX_RECORD_BYTES: usize = 8_192;
pub(super) const MAX_CONTRACT_BYTES: usize = 8_192;
pub(super) const FUTURE_RESERVE_BYTES: usize = 2_048;
pub(super) const PREFIX: &str = "house-renewal-offer:";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Closure {
    pub commit: CommitToken,
    pub action: DefinitionRef,
    pub at_ic: ClockStamp,
    pub at_utc: i64,
    pub reason: UnfundedCloseReason,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OfferRecord {
    pub format: u16,
    pub operation: CommitToken,
    pub action: DefinitionRef,
    pub offer: RenewalOffer,
    pub contract: ContractId,
    pub consent_hash: [u8; 32],
    pub created_at_ic: ClockStamp,
    pub created_at_utc: i64,
    #[serde(deserialize_with = "required_option")]
    pub closure: Option<Closure>,
    // Omitted for every historical format1 byte sequence. Explicit null is
    // invalid: absence and a present retry link are different wire shapes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_attempt"
    )]
    pub attempt: Option<RenewalAttemptV1>,
}
fn present_attempt<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<RenewalAttemptV1>, D::Error> {
    RenewalAttemptV1::deserialize(d).map(Some)
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
pub(super) fn key(operation: CommitToken) -> String {
    format!("{PREFIX}{}", operation.to_hex())
}
impl OfferRecord {
    pub(super) fn attempt(&self) -> RenewalAttemptV1 {
        self.attempt.unwrap_or(RenewalAttemptV1::Automatic)
    }
    pub(super) fn previous(&self) -> Option<CommitToken> {
        self.attempt().previous_operation()
    }
    pub(super) fn validate(&self) -> Result<(), ParticipantError> {
        self.offer.validate()?;
        let terms = &self.offer.terms;
        let shape = matches!(
            (self.format, self.attempt),
            (1, None)
                | (2, Some(RenewalAttemptV1::BuyerRetry { .. }))
                | (3, Some(RenewalAttemptV1::HolderAutomatic { .. }))
                | (4, Some(RenewalAttemptV1::HolderRetry { .. }))
        );
        if let Some(basis) = self.attempt().holder_basis() {
            basis.validate()?;
            if self.operation == basis.acceptance || self.operation == basis.ownership.operation {
                return Err(refused());
            }
        }
        if !shape
            || self.previous().is_some_and(|p| {
                p == self.operation
                    || p == self.offer.basis.predecessor_operation
                    || p == self.offer.basis.prior_quote
            })
            || !action_matches(&self.action, DRAFT_ACTION)
            || self.operation == self.offer.basis.predecessor_operation
            || self.operation == self.offer.basis.prior_quote
            || self.created_at_ic.clock != terms.not_before.clock
            || self.created_at_ic.at < terms.not_before.at
            || self.created_at_ic.at >= terms.expires_at.at
        {
            return Err(refused());
        }
        if let Some(close) = &self.closure
            && (!action_matches(&close.action, CLOSE_ACTION)
                || close.commit == self.operation
                || self.previous() == Some(close.commit)
                || close.commit == self.offer.basis.predecessor_operation
                || close.commit == self.offer.basis.prior_quote
                || close.at_ic.clock != self.created_at_ic.clock
                || close.at_ic.at < self.created_at_ic.at
                || close.at_utc < self.created_at_utc
                || (close.reason == UnfundedCloseReason::Expired
                    && close.at_ic.at < terms.expires_at.at))
        {
            return Err(refused());
        }
        let expected = contract(self)?;
        if expected.consent_hash != self.consent_hash {
            return Err(refused());
        }
        let reserve = if self.closure.is_none() {
            FUTURE_RESERVE_BYTES
        } else {
            0
        };
        let record_limit = if self.format >= 2 {
            MAX_RECORD_BYTES
        } else {
            MAX_RECORD_BYTES - reserve
        };
        if serde_json::to_vec(self)
            .map_err(|_| ParticipantError::Encoding)?
            .len()
            > record_limit
            || serde_json::to_vec(&expected)
                .map_err(|_| ParticipantError::Encoding)?
                .len()
                > MAX_CONTRACT_BYTES - reserve
        {
            return Err(refused());
        }
        if self.format >= 2 && self.closure.is_none() {
            self.require_close_room()?;
        }
        Ok(())
    }
    /// Only the new format2 uses actual complete future images instead of the
    /// historical fixed record reserve. This prevents a legal full format1
    /// offer becoming un-retryable solely because its new link consumes bytes.
    /// Total record/contract/envelope caps and F1 history caps do not increase.
    fn require_close_room(&self) -> Result<(), ParticipantError> {
        for reason in [UnfundedCloseReason::Declined, UnfundedCloseReason::Expired] {
            let mut terminal = self.clone();
            terminal.closure = Some(Closure {
                commit: CommitToken::from_bytes([255; 16]),
                // Fixed Draft/Close reference shapes differ only in their id;
                // account for that exact byte difference below. No authority
                // or actual closure is constructed by this size-only value.
                action: self.action.clone(),
                at_ic: ClockStamp {
                    clock: mush_core::ClockRef::WorldIc {
                        generation: u64::MAX,
                    },
                    at: i64::MIN,
                },
                at_utc: i64::MIN,
                reason,
            });
            let extra = CLOSE_ACTION.len().saturating_sub(DRAFT_ACTION.len());
            let terminal_contract = contract(&terminal)?;
            let bytes = serde_json::to_vec(&terminal)
                .map_err(|_| ParticipantError::Encoding)?
                .len();
            if bytes
                .checked_add(extra)
                .is_none_or(|n| n > MAX_RECORD_BYTES)
                || serde_json::to_vec(&terminal_contract)
                    .map_err(|_| ParticipantError::Encoding)?
                    .len()
                    > MAX_CONTRACT_BYTES
            {
                return Err(refused());
            }
            let writes = vec![
                crate::registry_filing::json_write(key(self.operation).into(), &terminal)?,
                crate::registry_filing::json_write(
                    crate::api::contract_key(self.contract),
                    &terminal_contract,
                )?,
            ];
            if serde_json::to_vec(&writes)
                .map_err(|_| ParticipantError::Encoding)?
                .len()
                .checked_add(extra)
                .is_none_or(|n| n > 32_768)
            {
                return Err(refused());
            }
        }
        Ok(())
    }
    pub(super) fn verify_documents(
        &self,
        snapshot: &EconomySnapshot,
    ) -> Result<(), ParticipantError> {
        self.validate()?;
        if snapshot.contract(self.contract) != Some(&contract(self)?)
            || snapshot.holds().any(
                |h| matches!(h.purpose,HoldPurpose::Escrow{contract} if contract==self.contract),
            )
        {
            return Err(refused());
        }
        Ok(())
    }
    fn decode(bytes: &[u8]) -> Result<Self, ParticipantError> {
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(refused());
        }
        // Typed first: duplicate known fields cannot disappear through Value.
        let record: Self = serde_json::from_slice(bytes).map_err(|_| refused())?;
        record.validate()?;
        let original: Value = serde_json::from_slice(bytes).map_err(|_| refused())?;
        if serde_json::to_value(&record).map_err(|_| ParticipantError::Encoding)? != original {
            return Err(refused());
        }
        Ok(record)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct OfferBook {
    rows: BTreeMap<CommitToken, OfferRecord>,
    contracts: BTreeMap<ContractId, CommitToken>,
    services: BTreeMap<CommitToken, super::service::ServiceRecord>,
    // The legacy holder uses generation1. A transferred holder has its own
    // automatic/retry chain within the same unchanged total inventory bound.
    predecessors: BTreeMap<(u64, u64, u64), CommitToken>,
    generation: u64,
    // Runtime-only: the old unfunded-only document path cannot mint this.
    combined_verified: bool,
}
impl OfferBook {
    pub(super) const fn combined_verified(&self) -> bool {
        self.combined_verified
    }
    pub(super) fn operations(&self) -> impl ExactSizeIterator<Item = CommitToken> + '_ {
        self.rows.keys().copied()
    }
    /// Any retained service closure needs the service action in this composition.
    pub(crate) fn has_services(&self) -> bool {
        !self.services.is_empty()
    }
    pub(super) fn service(&self, operation: CommitToken) -> Option<&super::service::ServiceRecord> {
        self.services.get(&operation)
    }
    pub(super) fn install_service(&mut self, record: super::service::ServiceRecord) {
        self.services.insert(record.operation, record);
    }
    pub(super) fn verify_offer(
        &self,
        row: &OfferRecord,
        snapshot: &EconomySnapshot,
    ) -> Result<(), ParticipantError> {
        verify_pair(row, snapshot, None, self.service(row.operation))
    }
    pub(super) fn get(&self, operation: CommitToken) -> Option<&OfferRecord> {
        self.rows.get(&operation)
    }
    pub(super) const fn generation(&self) -> u64 {
        self.generation
    }
    pub(super) fn next_generation(&self) -> Result<u64, ParticipantError> {
        self.generation.checked_add(1).ok_or_else(refused)
    }
    pub(super) fn admits(&self, row: &OfferRecord) -> Result<(), ParticipantError> {
        row.validate()?;
        let basis = &row.offer.basis;
        if self.rows.len() >= MAX_RECORDS
            || self.rows.contains_key(&row.operation)
            || self.contracts.contains_key(&row.contract)
        {
            return Err(refused());
        }
        let highest = self
            .predecessors
            .range(
                (
                    basis.licence,
                    row.attempt()
                        .holder_basis()
                        .map_or(1, |b| b.ownership.generation),
                    0,
                )
                    ..=(
                        basis.licence,
                        row.attempt()
                            .holder_basis()
                            .map_or(1, |b| b.ownership.generation),
                        u64::MAX,
                    ),
            )
            .next_back()
            .map(|(key, op)| (key.2, *op));
        match row.attempt() {
            RenewalAttemptV1::Automatic | RenewalAttemptV1::HolderAutomatic { .. } => {
                if highest.is_some_and(|(generation, _)| generation >= basis.predecessor_generation)
                {
                    return Err(refused());
                }
            }
            RenewalAttemptV1::BuyerRetry { previous_operation }
            | RenewalAttemptV1::HolderRetry {
                previous_operation, ..
            } => {
                if highest != Some((basis.predecessor_generation, previous_operation)) {
                    return Err(refused());
                }
            }
        }
        self.next_generation()?;
        Ok(())
    }
    /// Repeated at actual preparation and consuming bind. A mutable terminal
    /// contract cannot stand in for the exact replayed previous funding proof.
    pub(super) fn require_retry(
        &self,
        row: &OfferRecord,
        snapshot: &EconomySnapshot,
        receipts: &super::receipt::RenewalReceiptAudit,
    ) -> Result<(), ParticipantError> {
        let Some(previous) = row.previous() else {
            return Ok(());
        };
        receipts.require_complete().map_err(|_| refused())?;
        let before = self.rows.get(&previous).ok_or_else(refused)?;
        require_link(
            before,
            row,
            snapshot,
            Some(receipts),
            self.service(before.operation),
        )
    }
    pub(super) fn install(&mut self, row: OfferRecord, generation: u64) {
        self.contracts.insert(row.contract, row.operation);
        self.predecessors.insert(
            (
                row.offer.basis.licence,
                row.attempt()
                    .holder_basis()
                    .map_or(1, |b| b.ownership.generation),
                row.offer.basis.predecessor_generation,
            ),
            row.operation,
        );
        self.rows.insert(row.operation, row);
        self.generation = generation;
    }
    pub(crate) fn from_documents_with_resale(
        snapshot: &EconomySnapshot,
        documents: &[(String, Vec<u8>)],
        receipts: &super::receipt::RenewalReceiptAudit,
        resale: &crate::managed_paid_resale::book::PaidResaleBook,
    ) -> Result<Self, ParticipantError> {
        receipts.require_complete().map_err(|_| refused())?;
        Self::reconstruct(snapshot, documents, Some(receipts), Some(resale))
    }
    pub(super) fn require_holder_origin(
        row: &OfferRecord,
        resale: &crate::managed_paid_resale::book::PaidResaleBook,
    ) -> Result<(), ParticipantError> {
        if let Some(basis) = row.attempt().holder_basis() {
            basis.require_origin(
                resale,
                row.offer.basis.licence,
                row.offer.terms.buyer,
                row.offer.terms.charter,
                row.offer.basis.predecessor_generation,
                row.offer.basis.predecessor_operation,
                &row.created_at_ic,
                row.created_at_utc,
                crate::managed_paid_resale::PaidBirthRouteV1::House,
            )?;
        }
        Ok(())
    }
    fn reconstruct(
        snapshot: &EconomySnapshot,
        documents: &[(String, Vec<u8>)],
        receipts: Option<&super::receipt::RenewalReceiptAudit>,
        resale: Option<&crate::managed_paid_resale::book::PaidResaleBook>,
    ) -> Result<Self, ParticipantError> {
        if documents.len() > MAX_RECORDS * 2 {
            return Err(refused());
        }
        let mut histories = match receipts {
            Some(r) => r
                .receipts()
                .map_err(|_| refused())?
                .map(|h| (h.operation(), h))
                .collect::<BTreeMap<_, _>>(),
            None => BTreeMap::new(),
        };
        let mut book = Self::default();
        for (name, bytes) in documents
            .iter()
            .filter(|(k, _)| k.starts_with(super::service::PREFIX))
        {
            let record = super::service::ServiceRecord::decode(bytes)?;
            if book.services.len() == MAX_RECORDS
                || *name != super::service::key(record.operation)
                || book.services.insert(record.operation, record).is_some()
            {
                return Err(refused());
            }
        }
        for (name, bytes) in documents
            .iter()
            .filter(|(k, _)| !k.starts_with(super::service::PREFIX))
        {
            let row = OfferRecord::decode(bytes)?;
            if row.attempt().holder_basis().is_some() {
                Self::require_holder_origin(&row, resale.ok_or_else(refused)?)?;
            }
            if book.rows.len() == MAX_RECORDS
                || *name != key(row.operation)
                || book.rows.contains_key(&row.operation)
                || book.contracts.contains_key(&row.contract)
            {
                return Err(refused());
            }
            verify_pair(
                &row,
                snapshot,
                histories.remove(&row.operation),
                book.service(row.operation),
            )?;
            book.contracts.insert(row.contract, row.operation);
            book.rows.insert(row.operation, row);
        }
        if !histories.is_empty() || book.services.keys().any(|op| !book.rows.contains_key(op)) {
            return Err(refused());
        }
        for contract in snapshot.contracts() {
            if contract
                .consent
                .managed
                .as_ref()
                .and_then(|m| m.house_renewal_offer_marker())
                .is_some()
                && !book.contracts.contains_key(&contract.id)
            {
                return Err(refused());
            }
        }
        book.rebuild_links(snapshot, receipts)?;
        book.generation = u64::try_from(book.rows.len()).map_err(|_| refused())?;
        book.combined_verified = receipts.is_some();
        Ok(book)
    }
    fn rebuild_links(
        &mut self,
        snapshot: &EconomySnapshot,
        receipts: Option<&super::receipt::RenewalReceiptAudit>,
    ) -> Result<(), ParticipantError> {
        let mut roots = BTreeMap::new();
        let mut followers = BTreeMap::new();
        for row in self.rows.values() {
            let group = (
                row.offer.basis.licence,
                row.attempt()
                    .holder_basis()
                    .map_or(1, |b| b.ownership.generation),
                row.offer.basis.predecessor_generation,
            );
            if let Some(previous) = row.previous() {
                let before = self.rows.get(&previous).ok_or_else(refused)?;
                require_link(
                    before,
                    row,
                    snapshot,
                    receipts,
                    self.service(before.operation),
                )?;
                if followers.insert(previous, row.operation).is_some() {
                    return Err(refused());
                }
            } else if roots.insert(group, row.operation).is_some() {
                return Err(refused());
            }
        }
        let mut visited = BTreeSet::new();
        for (group, root) in roots {
            let mut current = root;
            loop {
                if !visited.insert(current) {
                    return Err(refused());
                }
                match followers.get(&current) {
                    Some(next) => current = *next,
                    None => break,
                }
            }
            self.predecessors.insert(group, current);
        }
        if visited.len() != self.rows.len() {
            return Err(refused());
        }
        Ok(())
    }
}

fn verify_pair(
    row: &OfferRecord,
    snapshot: &EconomySnapshot,
    history: Option<&super::RenewalHistory>,
    service: Option<&super::service::ServiceRecord>,
) -> Result<(), ParticipantError> {
    if let Some(history) = history {
        if row.closure.is_some() || service.is_some() || history.origin() != row {
            return Err(refused());
        }
        history
            .verify_documents(
                snapshot.contract(row.contract).ok_or_else(refused)?,
                snapshot
                    .hold(history.funding().hold())
                    .ok_or_else(refused)?,
            )
            .map_err(|_| refused())?;
        if snapshot
            .holds()
            .filter(|h| matches!(h.purpose,HoldPurpose::Escrow{contract} if contract==row.contract))
            .count()
            != 1
        {
            return Err(refused());
        }
    } else if let Some(service) = service {
        row.validate()?;
        let expected = super::service::contract(row, service)?;
        if snapshot.contract(row.contract) != Some(&expected)
            || snapshot.holds().any(
                |h| matches!(h.purpose,HoldPurpose::Escrow{contract} if contract==row.contract),
            )
        {
            return Err(refused());
        }
    } else {
        row.verify_documents(snapshot)?;
    }
    Ok(())
}

fn require_link(
    before: &OfferRecord,
    row: &OfferRecord,
    snapshot: &EconomySnapshot,
    receipts: Option<&super::receipt::RenewalReceiptAudit>,
    service: Option<&super::service::ServiceRecord>,
) -> Result<(), ParticipantError> {
    if row.previous() != Some(before.operation)
        || row.operation == before.operation
        || row.contract == before.contract
        || row.attempt().holder_basis() != before.attempt().holder_basis()
        || row.offer.basis != before.offer.basis
        || row.offer.terms.buyer != before.offer.terms.buyer
        || row.offer.terms.charter != before.offer.terms.charter
        || row.offer.terms.organization != before.offer.terms.organization
        || row.offer.terms.scope != before.offer.terms.scope
        || row.offer.terms.instrument != before.offer.terms.instrument
    {
        return Err(refused());
    }
    let history = match receipts {
        Some(r) => r.find(before.operation).map_err(|_| refused())?,
        None => None,
    };
    verify_pair(before, snapshot, history, service)?;
    let (at, utc, terminal_commit) = if let Some(h) = history {
        let terminal = h.terminal().ok_or_else(refused)?;
        if terminal.outcome() != RenewalOutcome::Refunded {
            return Err(refused());
        }
        (
            terminal.at_ic(),
            terminal.posted_at_utc(),
            terminal.commit(),
        )
    } else if let Some(record) = service {
        (
            record.closure.at_ic(),
            record.closure.posted_at_utc(),
            record.closure.commit(),
        )
    } else {
        let closure = before.closure.as_ref().ok_or_else(refused)?;
        (&closure.at_ic, closure.at_utc, closure.commit)
    };
    if row.operation == terminal_commit
        || row.created_at_ic.clock != at.clock
        || row.created_at_ic.at < at.at
        || row.created_at_utc < utc
    {
        return Err(refused());
    }
    Ok(())
}
