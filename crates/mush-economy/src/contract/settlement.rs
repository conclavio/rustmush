//! Inert preparation shared by the live settlement path and later closed E4
//! adapters. This is a checked plan/authorization pair, not a posted receipt,
//! durable reservation, current-world fence, or paid entitlement.

use super::{
    ContractAuth, ContractError, EconomicContract, check_settlement, resolve_outcome,
    resolved_settlement_plan,
};
use crate::engine::ValidateEnv;
use crate::plan::EconomyPlan;
use crate::snapshot::EconomySnapshot;

/// The plan and its authorization derive from one exact consent carrier,
/// qualifying escrow snapshot and resolved outcome. Neither can be replaced by a
/// caller. The borrow keeps that consent carrier immutable while the pair lives.
///
/// This does not freeze the snapshot or external facts. The live caller posts
/// synchronously through the existing engine validation/key gate. A future
/// composed adapter must additionally capture and recheck all current reads and
/// serialize its complete after-images before writer custody.
pub(crate) struct SettlementPreparation<'a> {
    plan: EconomyPlan,
    authorization: ContractAuth<'a>,
}

impl<'a> SettlementPreparation<'a> {
    pub(crate) fn plan(&self) -> &EconomyPlan {
        &self.plan
    }

    pub(crate) fn authorization(&self) -> &ContractAuth<'a> {
        &self.authorization
    }
}

/// Resolve once, then retain the live path's existing refusal ordering and run
/// every existing consent, condition and escrow check. No owner state changes,
/// key reservations, terminal records, receipts or notifications occur here.
pub(crate) fn prepare_settlement<'a>(
    contract: &'a EconomicContract,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> Result<SettlementPreparation<'a>, ContractError> {
    let outcome = resolve_outcome(contract, snapshot, env)?;
    check_settlement(contract, snapshot, env)?;
    let plan = resolved_settlement_plan(contract, snapshot, outcome)?;
    let authorization = ContractAuth::settling(contract, snapshot, outcome);
    Ok(SettlementPreparation {
        plan,
        authorization,
    })
}

#[cfg(test)]
mod tests;
