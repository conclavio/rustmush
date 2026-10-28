//! Permanent paired purchase/terminal rows. Only the closed provider constructs
//! new rows from actual E4 output and Roll's allocator result. Strict decoding
//! is structural and never substitutes for the cross-owner boot audit.

use mush_core::mechanics::{
    DefinitionId, DefinitionKey, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef};
use mush_economy::managed_purchase::{
    FundingOutput, FundingReceipt, PurchaseOutcome, TerminalOutput,
};
use mush_economy::{ContractId, HoldId, TransactionId};
use mush_roll::commerce::purchase::{PaidAllocation, PurchaseParticipants, PurchasePhase};
use mush_roll::commerce::{LicenceId, TermGeneration};
use serde::{Deserialize, Serialize};

use super::records::{MindQuote, Row};

pub(super) const FUND: &str = "purchase_original_mind";
pub(super) const GRANT: &str = "settle_original_mind";
pub(super) const REFUND: &str = "refund_original_mind";

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub(super) fn descriptor_for(
    action: &str,
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
    protocol: super::protocol::PurchaseProtocol,
) -> Result<mush_core::mechanics::DefinitionDescriptor, String> {
    let transition = match action {
        FUND => "fund",
        GRANT => "grant",
        REFUND => "refund",
        _ => return Err("unknown purchase action".into()),
    };
    if manifest.key().owner.as_str() != "tacsim"
        || manifest.key().kind.as_str() != "registry_manifest"
        || manifest.key().id.as_str() != "combat"
        || manifest.version() != 1
    {
        return Err("invalid purchase manifest identity".into());
    }
    super::versioned_descriptor(
        "action",
        action,
        protocol.version(),
        protocol.payload(transition),
    )
    .map(|d| {
        d.with_references(vec![
            manifest.clone(),
            participants.economy().clone(),
            participants.retention().clone(),
        ])
    })
    .map_err(error)
}

fn historical_fund(
    compatibility: DefinitionDigest,
    participants: &PurchaseParticipants,
    protocol: super::protocol::PurchaseProtocol,
) -> Result<DefinitionRef, String> {
    let manifest = DefinitionRef::new(
        DefinitionKey {
            owner: OwnerId::new("tacsim").map_err(error)?,
            kind: DefinitionKind::new("registry_manifest").map_err(error)?,
            id: DefinitionId::new("combat").map_err(error)?,
        },
        1,
        compatibility,
    )
    .map_err(error)?;
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(crate::NS).map_err(error)?,
            descriptor_for(FUND, &manifest, participants, protocol)?,
        )
        .map_err(error)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Purchase {
    pub(super) format: u16,
    pub(super) operation: CommitToken,
    pub(super) effect: DefinitionRef,
    pub(super) participants: PurchaseParticipants,
    pub(super) quote: MindQuote,
    pub(super) quote_digest: DefinitionDigest,
    pub(super) licence: LicenceId,
    pub(super) term: TermGeneration,
    pub(super) funding: FundingReceipt,
    pub(super) phase: PurchasePhase,
    pub(super) updated_by: CommitToken,
}

impl Purchase {
    pub(super) fn funded(
        quote: MindQuote,
        effect: DefinitionRef,
        participants: PurchaseParticipants,
        output: &FundingOutput,
        allocation: &PaidAllocation,
    ) -> Result<Self, String> {
        let value = Self {
            format: 1,
            operation: output.operation(),
            effect,
            participants,
            quote_digest: quote.digest()?,
            quote,
            licence: allocation.licence(),
            term: allocation.term(),
            funding: output.receipt().clone(),
            phase: PurchasePhase::Funded {},
            updated_by: output.operation(),
        };
        value.validate()?;
        value.matches_allocation(allocation)?;
        let marker = output.marker();
        if marker.operation() != value.operation
            || marker.effect() != &value.effect
            || marker.quote() != value.quote.publication
            || marker.quote_digest() != value.quote_digest
            || marker.product_digest() != value.quote.product_digest
        {
            return Err("economic funding differs from purchase subject".into());
        }
        Ok(value)
    }

    pub(super) fn matches_allocation(&self, row: &PaidAllocation) -> Result<(), String> {
        let q = &self.quote;
        if row.operation() != self.operation
            || row.participants() != &self.participants
            || row.licence() != self.licence
            || row.term() != self.term
            || row.quote() != q.publication
            || row.quote_digest() != self.quote_digest
            || row.product_digest() != q.product_digest
            || row.holder() != mush_economy::Holder::Character(q.buyer)
            || row.charter() != q.subject.charter
            || row.line() != q.subject.line
            || row.filed() != q.subject.filed
            || row.revision() != q.subject.revision
            || row.rating() != q.subject.rating
            || row.funding() != &self.funding
            || row.phase() != self.phase
            || row.updated_by() != self.updated_by
            || row.terms().seats().get() != q.product.seats
            || row.terms().class() != q.product.class
            || row.terms().form() != q.product.form
            || row.terms().transferable() != q.product.transferable
        {
            return Err("paired purchase records disagree".into());
        }
        Ok(())
    }

    pub(super) fn terminal(
        &self,
        output: &TerminalOutput,
    ) -> Result<(Self, PurchaseReceipt), String> {
        self.validate()?;
        if self.phase != (PurchasePhase::Funded {})
            || output.operation() != self.operation
            || output.commit() == self.operation
            || output.contract() != self.funding.contract()
            || output.hold() != self.funding.hold()
            || output.transaction() == self.funding.transaction()
        {
            return Err("terminal economics differ from funded purchase".into());
        }
        let mut after = self.clone();
        after.phase = match output.outcome() {
            PurchaseOutcome::Granted => PurchasePhase::Granted {
                commit: output.commit(),
            },
            PurchaseOutcome::Refunded => PurchasePhase::Refunded {
                commit: output.commit(),
            },
        };
        after.updated_by = output.commit();
        after.validate()?;
        let receipt = PurchaseReceipt {
            format: 1,
            operation: self.operation,
            commit: output.commit(),
            outcome: match output.outcome() {
                PurchaseOutcome::Granted => ReceiptOutcome::Granted {},
                PurchaseOutcome::Refunded => ReceiptOutcome::Refunded {},
            },
            contract: output.contract(),
            hold: output.hold(),
            transaction: output.transaction(),
            journal_seq: output.journal_seq(),
            licence: self.licence,
            term: self.term,
            quote_digest: self.quote_digest,
            product_digest: self.quote.product_digest,
            issued_at_ic: output.issued_at_ic().cloned(),
        };
        receipt.validate()?;
        Ok((after, receipt))
    }
}
impl Row for Purchase {
    const FAMILY: &'static str = "market_mind_purchase";
    const MAX_BYTES: usize = 32768;
    fn validate(&self) -> Result<(), String> {
        self.quote.validate()?;
        self.phase
            .validate(self.operation, self.updated_by)
            .map_err(error)?;
        if self.format != 1
            || self.term.get() != 1
            || self.operation == self.quote.publication
            || self.quote.digest()? != self.quote_digest
            || self.effect
                != historical_fund(
                    self.quote.subject.compatibility,
                    &self.participants,
                    self.quote.product.protocol(),
                )?
        {
            return Err("invalid permanent purchase identity".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ReceiptOutcome {
    Granted {},
    Refunded {},
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PurchaseReceipt {
    format: u16,
    operation: CommitToken,
    commit: CommitToken,
    outcome: ReceiptOutcome,
    contract: ContractId,
    hold: HoldId,
    transaction: TransactionId,
    journal_seq: u64,
    licence: LicenceId,
    term: TermGeneration,
    quote_digest: DefinitionDigest,
    product_digest: DefinitionDigest,
    // Explicit deserialize_with distinguishes absent from required JSON null.
    #[serde(deserialize_with = "required_stamp")]
    issued_at_ic: Option<ClockStamp>,
}
fn required_stamp<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<ClockStamp>, D::Error> {
    Option::<ClockStamp>::deserialize(d)
}
impl PurchaseReceipt {
    pub(super) fn operation(&self) -> CommitToken {
        self.operation
    }
    pub(super) fn verify_history(
        &self,
        row: &Purchase,
        history: &mush_economy::managed_purchase::PurchaseHistory,
    ) -> Result<(), String> {
        self.validate()?;
        let terminal = history.terminal().ok_or("terminal history is missing")?;
        let outcome = if terminal.granted() {
            ReceiptOutcome::Granted {}
        } else {
            ReceiptOutcome::Refunded {}
        };
        if self.operation != row.operation
            || self.commit != row.updated_by
            || self.commit != terminal.commit()
            || self.outcome != outcome
            || self.contract != row.funding.contract()
            || self.hold != row.funding.hold()
            || self.transaction != terminal.transaction()
            || self.journal_seq != terminal.journal_seq()
            || self.issued_at_ic.as_ref() != terminal.issued_at_ic()
            || self.licence != row.licence
            || self.term != row.term
            || self.quote_digest != row.quote_digest
            || self.product_digest != row.quote.product_digest
        {
            return Err("terminal receipt differs from actual purchase history".into());
        }
        Ok(())
    }
}
impl Row for PurchaseReceipt {
    const FAMILY: &'static str = "market_mind_purchase_receipt";
    const MAX_BYTES: usize = 8192;
    fn validate(&self) -> Result<(), String> {
        if self.format != 1 || self.operation == self.commit || self.term.get() != 1 {
            return Err("invalid terminal purchase receipt".into());
        }
        match (self.outcome, self.issued_at_ic.as_ref()) {
            (ReceiptOutcome::Granted {}, Some(stamp)) if matches!(stamp.clock, ClockRef::WorldIc { generation } if generation != 0) => {
                Ok(())
            }
            (ReceiptOutcome::Refunded {}, None) => Ok(()),
            _ => Err("terminal receipt clock differs from outcome".into()),
        }
    }
}
