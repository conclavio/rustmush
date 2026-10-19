//! The old native sale and a native escrow settlement are distinct proofs.
//! The transparent immediate variant preserves every format1 economic byte.
use crate::commerce::records::PublicationError;
use mush_core::{CommitToken, DefinitionDigest, ObjId};
use mush_economy::managed_charter_transfer::CharterPendingHistory;
use mush_economy::registry_filing::charter_transfer::CharterTransferAuditSubject;
use mush_economy::{AccountId, Amount, EconomyStore, InstrumentId};
use serde::{Deserialize, Serialize};

fn bad<T>(_: T) -> PublicationError {
    PublicationError::Mismatch
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingEconomic {
    pub(crate) pending: CharterPendingHistory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum TransferEconomic {
    Immediate(CharterTransferAuditSubject),
    Pending(PendingEconomic),
}

/// Borrowed facts from the selected native proof. This is never an immediate
/// receipt synthesized from an escrow settlement, or reusable authority.
pub(crate) struct TransferFacts<'a> {
    pub(crate) operation: CommitToken,
    pub(crate) charter: ObjId,
    pub(crate) offer: CommitToken,
    pub(crate) offer_digest: DefinitionDigest,
    pub(crate) seller: ObjId,
    pub(crate) buyer: ObjId,
    pub(crate) seller_account: AccountId,
    pub(crate) instrument: &'a InstrumentId,
    pub(crate) price: Amount,
    pub(crate) title_revision: u64,
    pub(crate) posted_at_utc: i64,
    pub(crate) journal_seq: u64,
}

impl TransferEconomic {
    pub(crate) fn format(&self) -> u16 {
        match self {
            Self::Immediate(_) => 1,
            Self::Pending(_) => 2,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        match self {
            Self::Immediate(row) => row.validate().map_err(bad),
            Self::Pending(row) => {
                let bytes = mush_core::canonical_json_bytes_with_limit(
                    &serde_json::to_value(&row.pending).map_err(bad)?,
                    mush_economy::managed_charter_transfer::MAX_PENDING_HISTORY_BYTES,
                )
                .map_err(bad)?;
                CharterPendingHistory::decode(&bytes).map_err(bad)?;
                if row.pending.terminal().is_none_or(|terminal| {
                    !terminal.decision().reason.settles() || terminal.title().is_none()
                }) {
                    return Err(PublicationError::Mismatch);
                }
                Ok(())
            }
        }
    }

    pub(crate) fn facts(&self) -> Result<TransferFacts<'_>, PublicationError> {
        self.validate()?;
        Ok(match self {
            Self::Immediate(row) => TransferFacts {
                operation: row.commit,
                charter: row.charter,
                offer: row.offer,
                offer_digest: row.offer_digest,
                seller: row.seller,
                buyer: row.buyer,
                seller_account: row.seller_account,
                instrument: &row.instrument,
                price: row.price,
                title_revision: row.title_revision,
                posted_at_utc: row.posted_at_utc,
                journal_seq: row.journal_seq,
            },
            Self::Pending(row) => {
                let consent = row.pending.consent();
                let terminal = row.pending.terminal().ok_or(PublicationError::Mismatch)?;
                TransferFacts {
                    operation: terminal.decision().operation,
                    charter: consent.terms.charter,
                    offer: consent.terms.offer,
                    offer_digest: consent.terms.offer_digest,
                    seller: consent.terms.seller,
                    buyer: consent.terms.buyer,
                    seller_account: consent.terms.seller_account,
                    instrument: &consent.terms.instrument,
                    price: consent.terms.price,
                    title_revision: consent.terms.title_revision,
                    posted_at_utc: terminal.decision().posted_at_utc,
                    journal_seq: terminal.journal_seq(),
                }
            }
        })
    }

    pub(crate) fn verify(&self, economy: &EconomyStore) -> Result<(), PublicationError> {
        self.validate()?;
        match self {
            Self::Immediate(row) => economy
                .charter_transfer_receipt_audit()
                .verify(row)
                .map_err(bad),
            Self::Pending(row) => {
                if economy
                    .pending_charter_history(row.pending.consent().operation)
                    .map_err(bad)?
                    != &row.pending
                {
                    return Err(PublicationError::Mismatch);
                }
                Ok(())
            }
        }
    }
}

impl From<CharterTransferAuditSubject> for TransferEconomic {
    fn from(row: CharterTransferAuditSubject) -> Self {
        Self::Immediate(row)
    }
}
