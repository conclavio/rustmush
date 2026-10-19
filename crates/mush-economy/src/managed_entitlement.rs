//! Closed six-recipient entitlement economics. These requests carry facts, not
//! authority; only an explicitly granted existing Economy owner participant can
//! prepare them. This module registers no action and publishes no entitlement.

use mush_core::participant::{OwnerPrepareContext, ParticipantError};
use mush_core::{ClockStamp, CommitToken};

use crate::contract::managed::ManagedConsent;
use crate::managed_purchase::{FundingOutput, FundingReceipt, PurchaseOutcome, TerminalOutput};

pub(crate) const NAMESPACE: &str = "market.mind.entitlement.v1";
pub(crate) const MAX_PAYOUTS: usize = 6;
pub(crate) const MAX_PARENT_CUTS: usize = 3;
const MAX_PARENT_PERMILLE: u16 = 300;
fn refused() -> ParticipantError {
    ParticipantError::refused("No entitlement purchase is available under these terms.")
}

pub(crate) mod subject;
pub use subject::{
    EntitlementProtocol, EntitlementPurchase, GrossRecipientCut, Recipient, RoyaltyRecipientCut,
    RoyaltyTitleRecipient,
};

#[derive(Clone, Debug)]
pub struct EntitlementTerminal {
    pub operation: CommitToken,
    pub purchase: EntitlementPurchase,
    pub funding: FundingReceipt,
}

/// Private construction prevents an original-purchase consumer from accepting
/// this result accidentally. Every field comes from the real E4 preparation.
#[derive(Clone, Debug)]
pub struct EntitlementFundingOutput(FundingOutput);
impl EntitlementFundingOutput {
    pub fn operation(&self) -> CommitToken {
        self.0.operation()
    }
    pub fn marker(&self) -> &ManagedConsent {
        self.0.marker()
    }
    pub fn receipt(&self) -> &FundingReceipt {
        self.0.receipt()
    }
}
#[derive(Clone, Debug)]
pub struct EntitlementTerminalOutput(TerminalOutput);
impl EntitlementTerminalOutput {
    pub fn operation(&self) -> CommitToken {
        self.0.operation()
    }
    pub fn commit(&self) -> CommitToken {
        self.0.commit()
    }
    pub fn outcome(&self) -> PurchaseOutcome {
        self.0.outcome()
    }
    pub fn contract(&self) -> crate::ContractId {
        self.0.contract()
    }
    pub fn hold(&self) -> crate::HoldId {
        self.0.hold()
    }
    pub fn transaction(&self) -> crate::TransactionId {
        self.0.transaction()
    }
    pub fn journal_seq(&self) -> u64 {
        self.0.journal_seq()
    }
    pub fn issued_at_ic(&self) -> Option<&ClockStamp> {
        self.0.issued_at_ic()
    }
}

pub(crate) fn prepare_funding(
    state: &crate::module::EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: EntitlementPurchase,
) -> Result<
    (
        EntitlementFundingOutput,
        crate::managed_purchase::PurchasePatch,
        Vec<mush_core::authoritative::ReplayWrite>,
    ),
    ParticipantError,
> {
    let subject = crate::managed_purchase::subject::PurchaseSubject::try_from(request)?;
    let (output, patch, writes) =
        crate::managed_purchase::install::prepare_images(state, context, subject)?;
    Ok((EntitlementFundingOutput(output), patch, writes))
}
pub(crate) fn prepare_terminal(
    state: &crate::module::EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: EntitlementTerminal,
    outcome: PurchaseOutcome,
) -> Result<
    (
        EntitlementTerminalOutput,
        crate::managed_purchase::PurchasePatch,
        Vec<mush_core::authoritative::ReplayWrite>,
    ),
    ParticipantError,
> {
    let request = crate::managed_purchase::subject::SubjectTerminal::try_from(request)?;
    let (output, patch, writes) = crate::managed_purchase::install::prepare_terminal_images(
        state, context, request, outcome,
    )?;
    Ok((EntitlementTerminalOutput(output), patch, writes))
}

pub(crate) use subject::EconomicRecipe;
