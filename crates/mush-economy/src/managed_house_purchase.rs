//! Closed organization-issued original house purchase. These requests carry facts, not
//! authority; only an explicitly granted existing Economy owner participant can
//! prepare them. This module registers no action and publishes no entitlement.

use mush_core::participant::{OwnerPrepareContext, ParticipantError};
use mush_core::{ClockStamp, CommitToken};

use crate::contract::managed::ManagedConsent;
use crate::managed_purchase::{FundingOutput, FundingReceipt, PurchaseOutcome, TerminalOutput};

pub(crate) const NAMESPACE: &str = "market.mind.house.v1";
pub(crate) const MAX_PAYOUTS: usize = 3;
fn refused() -> ParticipantError {
    ParticipantError::refused("No house purchase is available under these terms.")
}

pub(crate) mod subject;
pub use subject::HousePurchase;

#[derive(Clone, Debug)]
pub struct HouseTerminal {
    pub operation: CommitToken,
    pub purchase: HousePurchase,
    pub funding: FundingReceipt,
}

/// Private construction prevents an original-purchase consumer from accepting
/// this result accidentally. Every field comes from the real E4 preparation.
#[derive(Clone, Debug)]
pub struct HouseFundingOutput(FundingOutput);
impl HouseFundingOutput {
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
pub struct HouseTerminalOutput(TerminalOutput);
impl HouseTerminalOutput {
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
    request: HousePurchase,
) -> Result<
    (
        HouseFundingOutput,
        Box<crate::managed_purchase::PurchasePatch>,
        Vec<mush_core::authoritative::ReplayWrite>,
    ),
    ParticipantError,
> {
    let subject = crate::managed_purchase::subject::PurchaseSubject::try_from(request)?;
    let (output, patch, writes) =
        crate::managed_purchase::install::prepare_images(state, context, subject)?;
    Ok((HouseFundingOutput(output), patch, writes))
}
pub(crate) fn prepare_terminal(
    state: &crate::module::EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: HouseTerminal,
    outcome: PurchaseOutcome,
) -> Result<
    (
        HouseTerminalOutput,
        Box<crate::managed_purchase::PurchasePatch>,
        Vec<mush_core::authoritative::ReplayWrite>,
    ),
    ParticipantError,
> {
    let request = crate::managed_purchase::subject::SubjectTerminal::try_from(request)?;
    let (output, patch, writes) = crate::managed_purchase::install::prepare_terminal_images(
        state, context, request, outcome,
    )?;
    Ok((HouseTerminalOutput(output), patch, writes))
}
