//! Registered economy participant for the closed original mind-filing action.
//!
//! The request contains no plan, fee, issuer or arbitrary write. Composition
//! freezes those terms. It is usable only through the core's exact action-bound
//! participant handle; ordinary post/issue-title callers gain no new authority.

#[cfg(test)]
mod boot_tests;
pub mod charter_transfer;
mod house;
mod prepare;
mod receipt;
pub mod update;
pub use house::{
    HOUSE_FILING_ACTION, HOUSE_PUBLICATION_CAPABILITY, HouseFilingAttribution, RegistryHouseFiling,
};
pub use update::{
    RegistryCompatibleUpdate, RegistryHouseCompatibleUpdate, UpdateAuditSubject, UpdateKind,
};
#[cfg(test)]
mod tests;
pub use receipt::{InitialFilingAuditSubject, ReceiptAudit, ReceiptAuditError};

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::action::CommitToken;
use mush_core::authoritative::ReplayWrite;
use mush_core::ledger::BoundLedgerAppend;
use mush_core::mechanics::OwnerId;
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{AuthoritativeKey, ObjId, PreparedLedgerAppend, World};
use mush_org::{OrgScopeId, PrincipalId, SystemPrincipalId};
use serde_json::Value;
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::api::{WorldAuth, account_key, asset_key, scope_coherent};
use crate::bloom::{BoundBloomInsert, PreparedBloomInsert};
use crate::engine::{AuthEval, Delta, NoCustody, NoFacts, ValidateEnv};
use crate::fold::FoldState;
use crate::ids::{AccountId, TransactionId};
use crate::module::{ColdKey, EconomyStore, JOURNAL_LOG, TxRef, WindowTail};
use crate::plan::{AssetOp, EconomyPlan, PlanResult, Prepared, RequestedLine};
use crate::record::{
    Account, CatalogRef, EconomicAsset, EconomicCause, EconomicTransaction, IdempotencyKey,
    InstrumentDef,
};
use crate::snapshot::EconomySnapshot;

/// Frozen product terms supplied by actual module composition.
#[derive(Clone, Debug)]
pub struct InitialFilingPolicy {
    issuer: SystemPrincipalId,
    sink: AccountId,
    fee: Amount,
    catalog: CatalogRef,
}
impl InitialFilingPolicy {
    /// The exact issuer selected by trusted host composition. Reading the
    /// policy creates no title, grant, or permission to post a transaction.
    pub fn issuer(&self) -> &SystemPrincipalId {
        &self.issuer
    }
    /// Exact inert catalogue selected by the same trusted filing configuration.
    pub fn catalog(&self) -> &CatalogRef {
        &self.catalog
    }
    pub fn new(
        issuer: SystemPrincipalId,
        sink: AccountId,
        fee: Amount,
        catalog: CatalogRef,
    ) -> Result<Self, ParticipantError> {
        if fee.is_negative() {
            return Err(refused());
        }
        Ok(Self {
            issuer,
            sink,
            fee,
            catalog,
        })
    }
    /// Registration hashes these exact fixed terms into its definition.
    pub fn definition_payload(&self) -> Value {
        serde_json::json!({"format":1,"issuer":self.issuer,"sink":self.sink,
            "fee":self.fee,"catalog":self.catalog})
    }
}

/// Not a wire DTO. The closed Roll builder supplies the acknowledged inert
/// shell and exact recipient after resolving its real reservation receipt.
pub struct RegistryInitialFiling {
    pub shell: ObjId,
    pub payer: AccountId,
    pub owner: PrincipalId,
    pub scope: Option<OrgScopeId>,
}

/// Actual prepared E4 facts. Only this module constructs them, after validation
/// and exact journal/projection preparation; they cannot be deserialized.
#[derive(Clone, Debug)]
pub struct InitialFilingOutput {
    transaction: TransactionId,
    journal_seq: u64,
    title: EconomicAsset,
    payer: AccountId,
    sink: AccountId,
    fee: Amount,
    issuer: SystemPrincipalId,
    commit: CommitToken,
    house: Option<HouseFilingAttribution>,
}
impl InitialFilingOutput {
    pub fn house(&self) -> Option<&HouseFilingAttribution> {
        self.house.as_ref()
    }
    pub const fn transaction(&self) -> TransactionId {
        self.transaction
    }
    pub const fn journal_seq(&self) -> u64 {
        self.journal_seq
    }
    pub fn title(&self) -> &EconomicAsset {
        &self.title
    }
    pub const fn payer(&self) -> AccountId {
        self.payer
    }
    pub const fn sink(&self) -> AccountId {
        self.sink
    }
    pub const fn fee(&self) -> Amount {
        self.fee
    }
    pub fn issuer(&self) -> &SystemPrincipalId {
        &self.issuer
    }
    pub const fn commit_token(&self) -> CommitToken {
        self.commit
    }
}

#[derive(Clone, Debug)]
pub struct InitialFilingParticipant {
    policy: InitialFilingPolicy,
    renewals: Option<crate::managed_renewal::bridge::Configuration>,
    house_renewals: Option<crate::managed_house_renewal::bridge::Configuration>,
    compatible_update: Option<mush_core::DefinitionRef>,
    derived_update: Option<mush_core::DefinitionRef>,
    original_major: Option<mush_core::DefinitionRef>,
    house_update: Option<update::house::HouseUpdateConfig>,
    charter_transfer: Option<mush_core::DefinitionRef>,
    core_integration: Option<crate::core_integration::CoreIntegrationConfig>,
    core_refit: Option<crate::core_refit::CoreRefitConfig>,
    charter_pending: Option<crate::managed_charter_transfer::CharterPendingConfig>,
    paid_resale: Option<crate::managed_paid_resale::PaidResaleConfig>,
    coverage: Option<crate::managed_coverage::CoverageConfig>,
}
impl InitialFilingParticipant {
    pub fn new(policy: InitialFilingPolicy) -> Self {
        Self {
            policy,
            renewals: None,
            house_renewals: None,
            compatible_update: None,
            derived_update: None,
            original_major: None,
            house_update: None,
            charter_transfer: None,
            core_integration: None,
            core_refit: None,
            charter_pending: None,
            paid_resale: None,
            coverage: None,
        }
    }
}

pub(crate) fn register(
    reg: &mut mush_core::Registrar<'_>,
    policy: InitialFilingPolicy,
    action: mush_core::mechanics::DefinitionRef,
    purchases: Option<&[mush_core::mechanics::DefinitionRef; 3]>,
    entitlements: Option<&[[mush_core::mechanics::DefinitionRef; 3]; 2]>,
    house_action: Option<&mush_core::mechanics::DefinitionRef>,
    dependent_action: Option<&mush_core::mechanics::DefinitionRef>,
    renewals: Option<&crate::managed_renewal::bridge::Configuration>,
    compatible_update: Option<&mush_core::DefinitionRef>,
    charter_transfer: Option<&mush_core::DefinitionRef>,
) -> Result<(), mush_core::ModError> {
    register_with_core(
        reg,
        policy,
        action,
        purchases,
        entitlements,
        house_action,
        dependent_action,
        renewals,
        compatible_update,
        charter_transfer,
        None,
    )
}
pub(crate) fn register_with_core(
    reg: &mut mush_core::Registrar<'_>,
    policy: InitialFilingPolicy,
    action: mush_core::mechanics::DefinitionRef,
    purchases: Option<&[mush_core::mechanics::DefinitionRef; 3]>,
    entitlements: Option<&[[mush_core::mechanics::DefinitionRef; 3]; 2]>,
    house_action: Option<&mush_core::mechanics::DefinitionRef>,
    dependent_action: Option<&mush_core::mechanics::DefinitionRef>,
    renewals: Option<&crate::managed_renewal::bridge::Configuration>,
    compatible_update: Option<&mush_core::DefinitionRef>,
    charter_transfer: Option<&mush_core::DefinitionRef>,
    core_integration: Option<&crate::core_integration::CoreIntegrationConfig>,
) -> Result<(), mush_core::ModError> {
    register_with_pending(
        reg,
        policy,
        action,
        purchases,
        entitlements,
        house_action,
        dependent_action,
        renewals,
        compatible_update,
        charter_transfer,
        core_integration,
        None,
    )
}
pub(crate) fn register_with_pending(
    reg: &mut mush_core::Registrar<'_>,
    policy: InitialFilingPolicy,
    action: mush_core::mechanics::DefinitionRef,
    purchases: Option<&[mush_core::mechanics::DefinitionRef; 3]>,
    entitlements: Option<&[[mush_core::mechanics::DefinitionRef; 3]; 2]>,
    house_action: Option<&mush_core::mechanics::DefinitionRef>,
    dependent_action: Option<&mush_core::mechanics::DefinitionRef>,
    renewals: Option<&crate::managed_renewal::bridge::Configuration>,
    compatible_update: Option<&mush_core::DefinitionRef>,
    charter_transfer: Option<&mush_core::DefinitionRef>,
    core_integration: Option<&crate::core_integration::CoreIntegrationConfig>,
    charter_pending: Option<&crate::managed_charter_transfer::CharterPendingConfig>,
) -> Result<(), mush_core::ModError> {
    register_with_house_purchases(
        reg,
        policy,
        action,
        purchases,
        entitlements,
        house_action,
        dependent_action,
        renewals,
        compatible_update,
        charter_transfer,
        core_integration,
        charter_pending,
        None,
    )
}
pub(crate) fn register_with_house_purchases(
    reg: &mut mush_core::Registrar<'_>,
    policy: InitialFilingPolicy,
    action: mush_core::mechanics::DefinitionRef,
    purchases: Option<&[mush_core::mechanics::DefinitionRef; 3]>,
    entitlements: Option<&[[mush_core::mechanics::DefinitionRef; 3]; 2]>,
    house_action: Option<&mush_core::mechanics::DefinitionRef>,
    dependent_action: Option<&mush_core::mechanics::DefinitionRef>,
    renewals: Option<&crate::managed_renewal::bridge::Configuration>,
    compatible_update: Option<&mush_core::DefinitionRef>,
    charter_transfer: Option<&mush_core::DefinitionRef>,
    core_integration: Option<&crate::core_integration::CoreIntegrationConfig>,
    charter_pending: Option<&crate::managed_charter_transfer::CharterPendingConfig>,
    house_purchases: Option<&[mush_core::DefinitionRef; 3]>,
) -> Result<(), mush_core::ModError> {
    register_with_house_renewals(
        reg,
        policy,
        action,
        purchases,
        entitlements,
        house_action,
        dependent_action,
        renewals,
        compatible_update,
        charter_transfer,
        core_integration,
        charter_pending,
        house_purchases,
        None,
    )
}
pub(crate) fn register_with_house_renewals(
    reg: &mut mush_core::Registrar<'_>,
    policy: InitialFilingPolicy,
    action: mush_core::mechanics::DefinitionRef,
    purchases: Option<&[mush_core::mechanics::DefinitionRef; 3]>,
    entitlements: Option<&[[mush_core::mechanics::DefinitionRef; 3]; 2]>,
    house_action: Option<&mush_core::mechanics::DefinitionRef>,
    dependent_action: Option<&mush_core::mechanics::DefinitionRef>,
    renewals: Option<&crate::managed_renewal::bridge::Configuration>,
    compatible_update: Option<&mush_core::DefinitionRef>,
    charter_transfer: Option<&mush_core::DefinitionRef>,
    core_integration: Option<&crate::core_integration::CoreIntegrationConfig>,
    charter_pending: Option<&crate::managed_charter_transfer::CharterPendingConfig>,
    house_purchases: Option<&[mush_core::DefinitionRef; 3]>,
    house_renewals: Option<&crate::managed_house_renewal::bridge::Configuration>,
) -> Result<(), mush_core::ModError> {
    register_with_paid_resale(
        reg,
        policy,
        action,
        purchases,
        entitlements,
        house_action,
        dependent_action,
        renewals,
        compatible_update,
        charter_transfer,
        core_integration,
        charter_pending,
        house_purchases,
        house_renewals,
        None,
    )
}
pub(crate) fn register_with_paid_resale(
    reg: &mut mush_core::Registrar<'_>,
    policy: InitialFilingPolicy,
    action: mush_core::mechanics::DefinitionRef,
    purchases: Option<&[mush_core::mechanics::DefinitionRef; 3]>,
    entitlements: Option<&[[mush_core::mechanics::DefinitionRef; 3]; 2]>,
    house_action: Option<&mush_core::mechanics::DefinitionRef>,
    dependent_action: Option<&mush_core::mechanics::DefinitionRef>,
    renewals: Option<&crate::managed_renewal::bridge::Configuration>,
    compatible_update: Option<&mush_core::DefinitionRef>,
    charter_transfer: Option<&mush_core::DefinitionRef>,
    core_integration: Option<&crate::core_integration::CoreIntegrationConfig>,
    charter_pending: Option<&crate::managed_charter_transfer::CharterPendingConfig>,
    house_purchases: Option<&[mush_core::DefinitionRef; 3]>,
    house_renewals: Option<&crate::managed_house_renewal::bridge::Configuration>,
    paid_resale: Option<&crate::managed_paid_resale::PaidResaleConfig>,
) -> Result<(), mush_core::ModError> {
    register_with_core_refits(
        reg,
        policy,
        action,
        purchases,
        entitlements,
        house_action,
        dependent_action,
        renewals,
        compatible_update,
        charter_transfer,
        core_integration,
        charter_pending,
        house_purchases,
        house_renewals,
        paid_resale,
        None,
    )
}
pub(crate) fn register_with_core_refits(
    reg: &mut mush_core::Registrar<'_>,
    policy: InitialFilingPolicy,
    action: mush_core::mechanics::DefinitionRef,
    purchases: Option<&[mush_core::mechanics::DefinitionRef; 3]>,
    entitlements: Option<&[[mush_core::mechanics::DefinitionRef; 3]; 2]>,
    house_action: Option<&mush_core::mechanics::DefinitionRef>,
    dependent_action: Option<&mush_core::mechanics::DefinitionRef>,
    renewals: Option<&crate::managed_renewal::bridge::Configuration>,
    compatible_update: Option<&mush_core::DefinitionRef>,
    charter_transfer: Option<&mush_core::DefinitionRef>,
    core_integration: Option<&crate::core_integration::CoreIntegrationConfig>,
    charter_pending: Option<&crate::managed_charter_transfer::CharterPendingConfig>,
    house_purchases: Option<&[mush_core::DefinitionRef; 3]>,
    house_renewals: Option<&crate::managed_house_renewal::bridge::Configuration>,
    paid_resale: Option<&crate::managed_paid_resale::PaidResaleConfig>,
    core_refit: Option<&crate::core_refit::CoreRefitConfig>,
) -> Result<(), mush_core::ModError> {
    register_with_coverage(
        reg,
        policy,
        action,
        purchases,
        entitlements,
        house_action,
        dependent_action,
        renewals,
        compatible_update,
        charter_transfer,
        core_integration,
        charter_pending,
        house_purchases,
        house_renewals,
        paid_resale,
        core_refit,
        None,
        None,
    )
}
pub(crate) fn register_with_coverage(
    reg: &mut mush_core::Registrar<'_>,
    policy: InitialFilingPolicy,
    action: mush_core::mechanics::DefinitionRef,
    purchases: Option<&[mush_core::mechanics::DefinitionRef; 3]>,
    entitlements: Option<&[[mush_core::mechanics::DefinitionRef; 3]; 2]>,
    house_action: Option<&mush_core::mechanics::DefinitionRef>,
    dependent_action: Option<&mush_core::mechanics::DefinitionRef>,
    renewals: Option<&crate::managed_renewal::bridge::Configuration>,
    compatible_update: Option<&mush_core::DefinitionRef>,
    charter_transfer: Option<&mush_core::DefinitionRef>,
    core_integration: Option<&crate::core_integration::CoreIntegrationConfig>,
    charter_pending: Option<&crate::managed_charter_transfer::CharterPendingConfig>,
    house_purchases: Option<&[mush_core::DefinitionRef; 3]>,
    house_renewals: Option<&crate::managed_house_renewal::bridge::Configuration>,
    paid_resale: Option<&crate::managed_paid_resale::PaidResaleConfig>,
    core_refit: Option<&crate::core_refit::CoreRefitConfig>,
    coverage: Option<&crate::managed_coverage::CoverageConfig>,
    two_seat_house_purchases: Option<&[mush_core::DefinitionRef; 3]>,
) -> Result<(), mush_core::ModError> {
    register_with_derived_updates(
        reg,
        policy,
        action,
        purchases,
        entitlements,
        house_action,
        dependent_action,
        renewals,
        compatible_update,
        charter_transfer,
        core_integration,
        charter_pending,
        house_purchases,
        house_renewals,
        paid_resale,
        core_refit,
        coverage,
        two_seat_house_purchases,
        None,
    )
}
pub(crate) fn register_with_derived_updates(
    reg: &mut mush_core::Registrar<'_>,
    policy: InitialFilingPolicy,
    action: mush_core::mechanics::DefinitionRef,
    purchases: Option<&[mush_core::mechanics::DefinitionRef; 3]>,
    entitlements: Option<&[[mush_core::mechanics::DefinitionRef; 3]; 2]>,
    house_action: Option<&mush_core::mechanics::DefinitionRef>,
    dependent_action: Option<&mush_core::mechanics::DefinitionRef>,
    renewals: Option<&crate::managed_renewal::bridge::Configuration>,
    compatible_update: Option<&mush_core::DefinitionRef>,
    charter_transfer: Option<&mush_core::DefinitionRef>,
    core_integration: Option<&crate::core_integration::CoreIntegrationConfig>,
    charter_pending: Option<&crate::managed_charter_transfer::CharterPendingConfig>,
    house_purchases: Option<&[mush_core::DefinitionRef; 3]>,
    house_renewals: Option<&crate::managed_house_renewal::bridge::Configuration>,
    paid_resale: Option<&crate::managed_paid_resale::PaidResaleConfig>,
    core_refit: Option<&crate::core_refit::CoreRefitConfig>,
    coverage: Option<&crate::managed_coverage::CoverageConfig>,
    two_seat_house_purchases: Option<&[mush_core::DefinitionRef; 3]>,
    derived_update: Option<&mush_core::DefinitionRef>,
) -> Result<(), mush_core::ModError> {
    register_with_original_majors(
        reg,
        policy,
        action,
        purchases,
        entitlements,
        house_action,
        dependent_action,
        renewals,
        compatible_update,
        charter_transfer,
        core_integration,
        charter_pending,
        house_purchases,
        house_renewals,
        paid_resale,
        core_refit,
        coverage,
        two_seat_house_purchases,
        derived_update,
        None,
    )
}
pub(crate) fn register_with_original_majors(
    reg: &mut mush_core::Registrar<'_>,
    policy: InitialFilingPolicy,
    action: mush_core::mechanics::DefinitionRef,
    purchases: Option<&[mush_core::mechanics::DefinitionRef; 3]>,
    entitlements: Option<&[[mush_core::mechanics::DefinitionRef; 3]; 2]>,
    house_action: Option<&mush_core::mechanics::DefinitionRef>,
    dependent_action: Option<&mush_core::mechanics::DefinitionRef>,
    renewals: Option<&crate::managed_renewal::bridge::Configuration>,
    compatible_update: Option<&mush_core::DefinitionRef>,
    charter_transfer: Option<&mush_core::DefinitionRef>,
    core_integration: Option<&crate::core_integration::CoreIntegrationConfig>,
    charter_pending: Option<&crate::managed_charter_transfer::CharterPendingConfig>,
    house_purchases: Option<&[mush_core::DefinitionRef; 3]>,
    house_renewals: Option<&crate::managed_house_renewal::bridge::Configuration>,
    paid_resale: Option<&crate::managed_paid_resale::PaidResaleConfig>,
    core_refit: Option<&crate::core_refit::CoreRefitConfig>,
    coverage: Option<&crate::managed_coverage::CoverageConfig>,
    two_seat_house_purchases: Option<&[mush_core::DefinitionRef; 3]>,
    derived_update: Option<&mush_core::DefinitionRef>,
    original_major: Option<&mush_core::DefinitionRef>,
) -> Result<(), mush_core::ModError> {
    register_with_house_updates(
        reg,
        policy,
        action,
        purchases,
        entitlements,
        house_action,
        dependent_action,
        renewals,
        compatible_update,
        charter_transfer,
        core_integration,
        charter_pending,
        house_purchases,
        house_renewals,
        paid_resale,
        core_refit,
        coverage,
        two_seat_house_purchases,
        derived_update,
        original_major,
        None,
    )
}
pub(crate) fn register_with_house_updates(
    reg: &mut mush_core::Registrar<'_>,
    policy: InitialFilingPolicy,
    action: mush_core::mechanics::DefinitionRef,
    purchases: Option<&[mush_core::mechanics::DefinitionRef; 3]>,
    entitlements: Option<&[[mush_core::mechanics::DefinitionRef; 3]; 2]>,
    house_action: Option<&mush_core::mechanics::DefinitionRef>,
    dependent_action: Option<&mush_core::mechanics::DefinitionRef>,
    renewals: Option<&crate::managed_renewal::bridge::Configuration>,
    compatible_update: Option<&mush_core::DefinitionRef>,
    charter_transfer: Option<&mush_core::DefinitionRef>,
    core_integration: Option<&crate::core_integration::CoreIntegrationConfig>,
    charter_pending: Option<&crate::managed_charter_transfer::CharterPendingConfig>,
    house_purchases: Option<&[mush_core::DefinitionRef; 3]>,
    house_renewals: Option<&crate::managed_house_renewal::bridge::Configuration>,
    paid_resale: Option<&crate::managed_paid_resale::PaidResaleConfig>,
    core_refit: Option<&crate::core_refit::CoreRefitConfig>,
    coverage: Option<&crate::managed_coverage::CoverageConfig>,
    two_seat_house_purchases: Option<&[mush_core::DefinitionRef; 3]>,
    derived_update: Option<&mush_core::DefinitionRef>,
    original_major: Option<&mush_core::DefinitionRef>,
    house_update: Option<&update::house::HouseUpdateConfig>,
) -> Result<(), mush_core::ModError> {
    use mush_core::mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionValidationContext,
    };
    let bad = |error: &dyn std::fmt::Display| mush_core::ModError::msg(error.to_string());
    if action.key().owner.as_str() != "roll"
        || action.key().kind.as_str() != "action"
        || action.key().id.as_str() != "initial_filing"
        || action.version() != 1
    {
        return Err(mush_core::ModError::msg(
            "economy initial filing needs the exact Roll initial_filing action",
        ));
    }
    let kind = DefinitionKind::new("owner_participant").map_err(|e| bad(&e))?;
    let payload = policy.definition_payload();
    let expected = payload.clone();
    reg.definition_validator(
        kind.clone(),
        move |context: &DefinitionValidationContext<'_>| {
            if context.definition().payload() == &expected
                && context.definition().references().is_empty()
            {
                Ok(())
            } else {
                Err("initial filing participant terms differ from composition".into())
            }
        },
    )?;
    let definition = reg.definition(
        DefinitionDescriptor::new(
            kind,
            DefinitionId::new("initial_filing").map_err(|e| bad(&e))?,
            1,
            payload,
        )
        .map_err(|e| bad(&e))?,
    )?;
    let mut actions = std::collections::BTreeSet::from([action]);
    if let Some(house) = house_action {
        if house.key().owner.as_str() != "roll"
            || house.key().kind.as_str() != "action"
            || house.key().id.as_str() != HOUSE_FILING_ACTION
            || house.version() != 1
        {
            return Err(mush_core::ModError::msg(
                "house filing requires the exact Roll action",
            ));
        }
        actions.insert(house.clone());
    }
    if let Some(dependent) = dependent_action {
        if dependent.key().owner.as_str() != "roll"
            || dependent.key().kind.as_str() != "action"
            || dependent.key().id.as_str() != "dependent_initial_filing"
            || dependent.version() != 1
        {
            return Err(mush_core::ModError::msg(
                "dependent filing requires the exact Roll action",
            ));
        }
        actions.insert(dependent.clone());
    }
    if let Some(purchases) = purchases {
        for (reference, id) in purchases.iter().zip([
            "purchase_original_mind",
            "settle_original_mind",
            "refund_original_mind",
        ]) {
            if reference.key().owner.as_str() != "market"
                || reference.key().kind.as_str() != "action"
                || reference.key().id.as_str() != id
                || !matches!(reference.version(), 1 | 2)
                || reference.version() != purchases[0].version()
            {
                return Err(mush_core::ModError::msg(
                    "purchase participant requires the exact ordered Market actions",
                ));
            }
            actions.insert(reference.clone());
        }
    }
    if let Some([original, derived]) = entitlements {
        for (references, version, ids) in [
            (
                original,
                3,
                [
                    "purchase_original_mind",
                    "settle_original_mind",
                    "refund_original_mind",
                ],
            ),
            (
                derived,
                1,
                [
                    "purchase_derived_mind",
                    "settle_derived_mind",
                    "refund_derived_mind",
                ],
            ),
        ] {
            for (reference, id) in references.iter().zip(ids) {
                if reference.key().owner.as_str() != "market"
                    || reference.key().kind.as_str() != "action"
                    || reference.key().id.as_str() != id
                    || reference.version() != version
                {
                    return Err(mush_core::ModError::msg(
                        "entitlement participant requires the exact ordered Market protocols",
                    ));
                }
                actions.insert(reference.clone());
            }
        }
    }
    if two_seat_house_purchases.is_some() && house_purchases.is_none() {
        return Err(mush_core::ModError::msg(
            "two-seat House purchases require the one-seat prerequisite",
        ));
    }
    for (version, references) in [(1, house_purchases), (2, two_seat_house_purchases)] {
        let Some(references) = references else {
            continue;
        };
        for (reference, id) in references.iter().zip([
            "purchase_house_mind",
            "settle_house_mind",
            "refund_house_mind",
        ]) {
            if reference.key().owner.as_str() != "market"
                || reference.key().kind.as_str() != "action"
                || reference.key().id.as_str() != id
                || reference.version() != version
            {
                return Err(mush_core::ModError::msg(
                    "house purchase requires the exact ordered Market actions",
                ));
            }
            actions.insert(reference.clone());
        }
    }
    if let Some(renewals) = renewals {
        renewals.validate(&definition)?;
        actions.extend(renewals.actions.iter().cloned());
        actions.extend(renewals.service_close.iter().cloned());
    }
    if let Some(house_renewals) = house_renewals {
        house_renewals.validate(&definition)?;
        actions.extend(house_renewals.actions.iter().cloned());
        actions.extend(house_renewals.service_close.iter().cloned());
    }
    if let Some(action) = compatible_update {
        if policy.fee.minor() <= 0
            || action.key().owner.as_str() != "roll"
            || action.key().kind.as_str() != "action"
            || action.key().id.as_str() != update::ACTION_ID
            || action.version() != 1
        {
            return Err(mush_core::ModError::msg(
                "compatible updates require the exact action and positive filing fee",
            ));
        }
        actions.insert(action.clone());
    }
    if let Some(action) = derived_update {
        if policy.fee.minor() <= 0
            || action.key().owner.as_str() != "roll"
            || action.key().kind.as_str() != "action"
            || action.key().id.as_str() != update::DERIVED_ACTION_ID
            || action.version() != 1
        {
            return Err(mush_core::ModError::msg(
                "derived updates require the exact action and positive filing fee",
            ));
        }
        actions.insert(action.clone());
    }
    if let Some(action) = original_major {
        if policy.fee.minor() <= 0
            || action.key().owner.as_str() != "roll"
            || action.key().kind.as_str() != "action"
            || action.key().id.as_str() != update::MAJOR_ACTION_ID
            || action.version() != 1
        {
            return Err(mush_core::ModError::msg(
                "original majors require the exact action and positive filing fee",
            ));
        }
        actions.insert(action.clone());
    }
    if let Some(config) = house_update {
        let action = &config.action;
        if policy.fee.minor() <= 0
            || action.key().owner.as_str() != "roll"
            || action.key().kind.as_str() != "action"
            || action.key().id.as_str() != update::HOUSE_ACTION_ID
            || action.version() != 1
        {
            return Err(mush_core::ModError::msg(
                "House updates require the exact action and positive filing fee",
            ));
        }
        actions.insert(action.clone());
    }
    if let Some(action) = charter_transfer {
        if action.key().owner.as_str() != "roll"
            || action.key().kind.as_str() != "action"
            || action.key().id.as_str() != charter_transfer::ACTION_ID
            || action.version() != 1
        {
            return Err(mush_core::ModError::msg(
                "charter transfer requires the exact Roll action",
            ));
        }
        actions.insert(action.clone());
    }
    if let Some(config) = charter_pending {
        config.validate().map_err(|e| bad(&e))?;
        if config.issuer != policy.issuer
            || config.catalog != policy.catalog
            || charter_transfer.is_none()
        {
            return Err(mush_core::ModError::msg(
                "pending charter requires exact filing/title policy and immediate transfer composition",
            ));
        }
        actions.extend([
            config.fund.clone(),
            config.settle.clone(),
            config.refund.clone(),
        ]);
        actions.extend(config.revoke.iter().cloned());
    }
    if let Some(config) = coverage {
        config.validate().map_err(|e| bad(&e))?;
        // The registered Tacsim producer owns qualification and host evidence;
        // this existing E4 slot owns all premium, liability and title effects.
        actions.extend(config.actions.iter().cloned());
    }
    if let Some(config) = paid_resale {
        config.validate().map_err(|e| bad(&e))?;
        if config.issuer != policy.issuer {
            return Err(mush_core::ModError::msg(
                "paid resale requires exact configured monetary issuer",
            ));
        }
        // Offer is Market-only; the four monetary transitions reuse this owner slot.
        actions.extend(config.actions.iter().skip(1).cloned());
        actions.extend(config.revoke.iter().cloned());
    }
    if let Some(config) = core_integration {
        actions.extend(crate::core_integration::bridge::register(reg, config)?);
    }
    if let Some(config) = core_refit {
        if core_integration.is_none() {
            return Err(mush_core::ModError::msg(
                "Core refit requires actual native manufacture history",
            ));
        }
        actions.extend(crate::core_refit::bridge::register(reg, config)?);
    }
    reg.owner_participant(
        definition,
        actions,
        InitialFilingParticipant {
            policy,
            renewals: renewals.cloned(),
            house_renewals: house_renewals.cloned(),
            compatible_update: compatible_update.cloned(),
            derived_update: derived_update.cloned(),
            original_major: original_major.cloned(),
            house_update: house_update.cloned(),
            charter_transfer: charter_transfer.cloned(),
            core_integration: core_integration.cloned(),
            core_refit: core_refit.cloned(),
            charter_pending: charter_pending.cloned(),
            paid_resale: paid_resale.cloned(),
            coverage: coverage.cloned(),
        },
    )
}

fn refused() -> ParticipantError {
    ParticipantError::refused("initial filing economy inputs are unavailable or changed")
}

struct FilingAuth<'a> {
    world: WorldAuth<'a>,
    filer: PrincipalId,
    issuer: PrincipalId,
    shell: ObjId,
    owner: PrincipalId,
    scope: Option<OrgScopeId>,
    catalog: CatalogRef,
    payer: AccountId,
    sink: AccountId,
    fee: Amount,
}
impl AuthEval for FilingAuth<'_> {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        if actor != &self.issuer {
            return false;
        }
        ((account.id == self.payer && delta.checked_neg() == Some(self.fee))
            || (account.id == self.sink && delta == self.fee))
            && self.world.authorize(&self.filer, account, delta)
    }
    fn authorize_asset(
        &self,
        actor: &PrincipalId,
        asset: Option<&EconomicAsset>,
        op: &AssetOp,
    ) -> bool {
        actor == &self.issuer
            && asset.is_none()
            && matches!(op,
            AssetOp::Title { object, owner, scope, catalog }
                if *object == self.shell && *owner == self.owner && *scope == self.scope
                    && catalog.as_ref() == Some(&self.catalog))
    }
}

impl InitialFilingParticipant {
    fn plan(&self, commit: CommitToken, request: &RegistryInitialFiling) -> EconomyPlan {
        self.plan_for(commit, request, None)
    }
    fn plan_for(
        &self,
        commit: CommitToken,
        request: &RegistryInitialFiling,
        house: Option<&HouseFilingAttribution>,
    ) -> EconomyPlan {
        let lines = if self.policy.fee == Amount::ZERO {
            vec![]
        } else {
            // Nonnegative policy was checked at composition, so its negation
            // cannot overflow (Amount is signed minor units).
            vec![
                RequestedLine {
                    account: request.payer,
                    delta: Amount::from_minor(-self.policy.fee.minor()),
                    memo: "mind filing".into(),
                },
                RequestedLine {
                    account: self.policy.sink,
                    delta: self.policy.fee,
                    memo: "mind filing".into(),
                },
            ]
        };
        let namespace = if house.is_some() {
            house::NAMESPACE
        } else {
            receipt::FILING_NAMESPACE
        };
        let correlation = match house {
            Some(house) => {
                receipt::house_correlation(commit, request.payer, self.policy.sink, house.curator())
            }
            None => receipt::correlation(commit, request.payer, self.policy.sink),
        };
        let mut plan = EconomyPlan::new(
            IdempotencyKey::new(namespace, commit.to_hex()),
            PrincipalId::System(self.policy.issuer.clone()),
            EconomicCause::Ruleset {
                namespace: namespace.into(),
                correlation: correlation.into(),
            },
            lines,
        );
        plan.description = if house.is_some() {
            "Original scoped house mind filing"
        } else {
            "Original private mind filing"
        }
        .into();
        plan.asset_ops.push(AssetOp::Title {
            object: request.shell,
            owner: request.owner.clone(),
            scope: request.scope,
            catalog: Some(self.policy.catalog.clone()),
        });
        plan
    }

    fn validate(
        &self,
        state: &EconomyStore,
        world: &World,
        filer: ObjId,
        request: &RegistryInitialFiling,
        plan: &EconomyPlan,
    ) -> Result<Prepared, ParticipantError> {
        self.validate_for(state, world, filer, request, plan, None)
    }
    fn validate_for(
        &self,
        state: &EconomyStore,
        world: &World,
        filer: ObjId,
        request: &RegistryInitialFiling,
        plan: &EconomyPlan,
        house: Option<&HouseFilingAttribution>,
    ) -> Result<Prepared, ParticipantError> {
        match house {
            Some(house) => house::check(world, state, filer, request, house, self.policy.fee)?,
            None if request.scope.is_some() || request.owner != PrincipalId::Object(filer) => {
                return Err(refused());
            }
            None => {}
        }
        if state.frozen
            || !state.is_title_issuer(&self.policy.issuer)
            || request.payer == self.policy.sink
            || request.shell.0 == 0
            || world
                .get(request.shell)
                .is_none_or(|object| object.flags.contains(mush_core::Flags::GOING))
            || world
                .get(filer)
                .is_none_or(|object| object.flags.contains(mush_core::Flags::GOING))
            || !scope_coherent(world, request.scope, &request.owner)
            || !(WorldAuth { w: world }).authorize_title_receipt(
                &PrincipalId::Object(filer),
                &request.owner,
                request.scope,
            )
        {
            return Err(refused());
        }
        let auth = FilingAuth {
            world: WorldAuth { w: world },
            filer: PrincipalId::Object(filer),
            issuer: PrincipalId::System(self.policy.issuer.clone()),
            shell: request.shell,
            owner: request.owner.clone(),
            scope: request.scope,
            catalog: self.policy.catalog.clone(),
            payer: request.payer,
            sink: self.policy.sink,
            fee: self.policy.fee,
        };
        let mut spent = BTreeMap::new();
        if self.policy.fee != Amount::ZERO
            && let Some(window) = state
                .snapshot
                .account(request.payer)
                .and_then(|a| a.approval.as_ref())
                .and_then(|rule| rule.window.as_ref())
        {
            spent.insert(
                request.payer,
                state.spent_in_window(request.payer, world.now(), window.period_secs),
            );
        }
        let gate = state.key_gate();
        let env = ValidateEnv {
            auth: &auth,
            custody: &NoCustody,
            facts: &NoFacts,
            now: world.now(),
            ic_now: world.game_now_unix(),
            next_hold_id: state.next_hold,
            spent,
            keys: &gate,
        };
        match crate::engine::validate(plan, &state.snapshot, &env) {
            PlanResult::Post(prepared) => Ok(*prepared),
            // Approval is not inherited from an arbitrary post or title command.
            // A separately registered exact filing-approval service is required.
            PlanResult::NeedsApproval(_)
            | PlanResult::AlreadyPosted(_)
            | PlanResult::Refused(_) => Err(refused()),
        }
    }
}

/// All serialization, exact read fences, index arithmetic and heap-owned
/// post-images are complete before this value reaches writer custody.
pub struct InitialFilingPatch {
    house: Option<HouseFilingAttribution>,
    clock: mush_core::ClockRef,
    request: RegistryInitialFiling,
    plan: EconomyPlan,
    validated: Prepared,
    before_meta: MetaFence,
    before_posts: u64,
    before_accounts: Vec<Account>,
    before_instruments: Vec<InstrumentDef>,
    before_window: Option<WindowTail>,
    next_window: Option<WindowTail>,
    journal: PreparedLedgerAppend,
    bloom: PreparedBloomInsert,
    delta: Delta,
    transaction: EconomicTransaction,
    next_transaction: u64,
    next_posts: u64,
    chunk: u64,
    fold: Box<InitialFold>,
}

/// The exact fields whose prepared meta image will be written. Comparing this
/// typed fence at bind performs no serialization and cannot ignore an allocator
/// or doc-authority change merely because the journal tail stayed put.
pub(crate) struct MetaFence {
    counters: [u64; 11],
    clocks: [i64; 3],
    flags: [bool; 2],
    boot_frozen: Vec<AccountId>,
}
impl MetaFence {
    fn counters(state: &EconomyStore) -> [u64; 11] {
        [
            state.next_account,
            state.next_transaction,
            state.next_vault,
            state.next_approval,
            state.next_hold,
            state.next_obligation,
            state.next_cadence,
            state.next_escalation,
            state.next_contract,
            state.sweep_contract_cursor,
            state.idem_backfill_txid,
        ]
    }
    pub(crate) fn capture(state: &EconomyStore) -> Self {
        Self {
            counters: Self::counters(state),
            clocks: [
                state.last_sweep_real,
                state.last_sweep_ic,
                state.last_boot_real,
            ],
            flags: [state.idem_backfilled, state.frozen],
            boot_frozen: state.boot_frozen.clone(),
        }
    }
    pub(crate) fn matches(&self, state: &EconomyStore) -> bool {
        self.counters == Self::counters(state)
            && self.clocks
                == [
                    state.last_sweep_real,
                    state.last_sweep_ic,
                    state.last_boot_real,
                ]
            && self.flags == [state.idem_backfilled, state.frozen]
            && self.boot_frozen == state.boot_frozen
    }
}

struct InitialFold {
    covered: Option<u64>,
    max_txid: u64,
    accounts: Vec<(AccountId, i128, u64, u64)>,
    title: EconomicAsset,
    receipt: InitialFilingAuditSubject,
    lineage: charter_transfer::lineage::Lineage,
}
impl InitialFold {
    fn prepare(state: &FoldState, tx: &EconomicTransaction) -> Result<Self, ParticipantError> {
        let receipt = receipt::subject_of(tx).map_err(|_| refused())?;
        let lineage = state
            .charter_transfer_receipts
            .prepare_birth(tx)
            .map_err(|_| refused())?;
        let [effect] = tx.asset_effects.as_slice() else {
            return Err(refused());
        };
        if state.journal_assets.contains_key(&effect.object)
            || state.created_count.contains_key(&effect.object)
        {
            return Err(refused());
        }
        let mut accounts = Vec::with_capacity(tx.lines.len());
        for line in &tx.lines {
            let running = state
                .running
                .get(&line.account)
                .copied()
                .unwrap_or(0)
                .checked_add(line.delta.widen())
                .ok_or_else(refused)?;
            let touches = state
                .touches
                .get(&line.account)
                .copied()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(refused)?;
            accounts.push((line.account, running, tx.journal_seq, touches));
        }
        Ok(Self {
            covered: Some(tx.journal_seq),
            max_txid: state.max_txid.max(tx.id.0),
            accounts,
            title: effect.post.clone(),
            receipt,
            lineage,
        })
    }
    fn apply(self, state: &mut FoldState) {
        for (id, running, seq, touches) in self.accounts {
            state.running.insert(id, running);
            state.last_touch.insert(id, seq);
            state.touches.insert(id, touches);
        }
        state.created_count.insert(self.title.object, 1);
        state.journal_assets.insert(self.title.object, self.title);
        state.max_txid = self.max_txid;
        state.covered_seq = self.covered;
        state.filing_receipts.install_prepared(self.receipt);
        state
            .charter_transfer_receipts
            .install_lineage(self.lineage);
    }
}

pub(crate) fn json_write<T: serde::Serialize>(
    key: SmolStr,
    row: &T,
) -> Result<ReplayWrite, ParticipantError> {
    let value = serde_json::to_value(row).map_err(|_| ParticipantError::Encoding)?;
    Ok(ReplayWrite::ModuleJsonSet {
        namespace: OwnerId::new(crate::module::NS).map_err(|_| refused())?,
        key: AuthoritativeKey::parse_canonical(key.as_str()).map_err(|_| refused())?,
        value,
    })
}

impl InitialFilingParticipant {
    fn prepare_images(
        &self,
        state: &EconomyStore,
        world: &World,
        filer: ObjId,
        commit: CommitToken,
        request: RegistryInitialFiling,
    ) -> Result<(InitialFilingOutput, InitialFilingPatch, Vec<ReplayWrite>), ParticipantError> {
        self.prepare_filing_images(state, world, filer, commit, request, None)
    }
    fn prepare_filing_images(
        &self,
        state: &EconomyStore,
        world: &World,
        filer: ObjId,
        commit: CommitToken,
        request: RegistryInitialFiling,
        house: Option<HouseFilingAttribution>,
    ) -> Result<(InitialFilingOutput, InitialFilingPatch, Vec<ReplayWrite>), ParticipantError> {
        if !state.fold.filing_receipts.may_add(commit) {
            return Err(refused());
        }
        let plan = match &house {
            Some(house) => self.plan_for(commit, &request, Some(house)),
            None => self.plan(commit, &request),
        };
        let validated = match &house {
            Some(house) => self.validate_for(state, world, filer, &request, &plan, Some(house))?,
            None => self.validate(state, world, filer, &request, &plan)?,
        };
        let id = TransactionId(state.next_transaction);
        let next_transaction = state.next_transaction.checked_add(1).ok_or_else(refused)?;
        let next_posts = state.posts_since_boot.checked_add(1).ok_or_else(refused)?;
        let seq = state.journal.next_seq(JOURNAL_LOG);
        let (delta, transaction) =
            crate::engine::apply_delta(&state.snapshot, &validated, id, seq, world.now());
        if !delta.holds.is_empty()
            || delta.consumed_approval.is_some()
            || delta.reversed.is_some()
            || !transaction.custody_effects.is_empty()
            || transaction.asset_effects.len() != 1
        {
            return Err(refused());
        }
        let title = delta.assets.first().cloned().ok_or_else(refused)?;
        let fold = Box::new(InitialFold::prepare(&state.fold, &transaction)?);
        let journal = state
            .journal
            .prepare_append(
                JOURNAL_LOG,
                serde_json::to_value(&transaction).map_err(|_| ParticipantError::Encoding)?,
            )
            .map_err(|_| refused())?;
        let mut writes = vec![
            journal
                .authoritative_replay_write(OwnerId::new(crate::module::NS).map_err(|_| refused())?)
                .map_err(|_| ParticipantError::Encoding)?,
        ];
        for account in &delta.accounts {
            writes.push(json_write(account_key(account.id), account)?);
        }
        writes.push(json_write(asset_key(title.object), &title)?);
        let key_doc = crate::keys::IdemDoc::of(&transaction);
        writes.push(json_write(
            crate::keys::idem_key(&transaction.idempotency),
            &key_doc,
        )?);
        let before_meta = MetaFence::capture(state);
        let mut next_meta = state.meta_doc();
        let meta = next_meta
            .as_object_mut()
            .ok_or(ParticipantError::Encoding)?;
        meta.insert("next_transaction_id".into(), Value::from(next_transaction));
        writes.push(json_write(crate::api::META_KEY.into(), &next_meta)?);
        let mut before_accounts = Vec::with_capacity(2);
        let mut before_instruments = Vec::with_capacity(2);
        for account in [request.payer, self.policy.sink] {
            let account = state
                .snapshot
                .account(account)
                .cloned()
                .ok_or_else(refused)?;
            let instrument = state
                .snapshot
                .instrument(&account.instrument)
                .cloned()
                .ok_or_else(refused)?;
            before_accounts.push(account);
            before_instruments.push(instrument);
        }
        let before_window = state.debit_window.get(&request.payer).cloned();
        let mut windows = BTreeMap::new();
        if let Some(window) = &before_window {
            windows.insert(request.payer, window.clone());
        }
        crate::module::note_window_debits(&mut windows, &transaction);
        let next_window = if crate::module::account_windowed(&state.snapshot, request.payer) {
            windows.remove(&request.payer)
        } else {
            None
        };
        let bloom = state
            .key_filter
            .prepare_insert(&transaction.idempotency)
            .ok_or_else(refused)?;
        let output = InitialFilingOutput {
            transaction: id,
            journal_seq: seq,
            title,
            payer: request.payer,
            sink: self.policy.sink,
            fee: self.policy.fee,
            issuer: self.policy.issuer.clone(),
            commit,
            house: house.clone(),
        };
        let patch = InitialFilingPatch {
            house,
            clock: world.world_ic_now().clock,
            request,
            plan,
            validated,
            before_meta,
            before_posts: state.posts_since_boot,
            before_accounts,
            before_instruments,
            before_window,
            next_window,
            journal,
            bloom,
            delta,
            transaction,
            next_transaction,
            next_posts,
            chunk: state.journal.open_chunk(JOURNAL_LOG),
            fold,
        };
        Ok((output, patch, writes))
    }
}

/// Closed requests to the one registered EconomyStore slot. Keeping the
/// original filing request/output wrappers avoids sharing money authority
/// through a second adapter or generic plan callback.
pub enum EconomyRequest {
    CoverageBind(crate::managed_coverage::CoverageBindRequest),
    CoverageTerminal(crate::managed_coverage::CoverageTerminalRequest),
    PaidResaleAcceptance(Box<crate::managed_paid_resale::PaidResaleAcceptanceRequest>),
    PaidResaleTerminal(crate::managed_paid_resale::PaidResaleTerminalRequest),
    HouseFunding(crate::managed_house_purchase::HousePurchase),
    HouseGrant(crate::managed_house_purchase::HouseTerminal),
    HouseRefund(crate::managed_house_purchase::HouseTerminal),
    CharterPendingFunding(crate::managed_charter_transfer::CharterPendingFundingRequest),
    CharterPendingTerminal(crate::managed_charter_transfer::CharterPendingTerminalRequest),
    CharterTransfer(charter_transfer::RegistryCharterTransfer),
    CoreFunding(crate::core_integration::CoreFundingRequest),
    CoreCompletion(crate::core_integration::CoreCompletionRequest),
    CoreCancellation(crate::core_integration::CoreCancellationRequest),
    RefitFunding(crate::core_refit::RefitFundingRequest),
    RefitInstallation(crate::core_refit::RefitInstallationRequest),
    RefitCancellation(crate::core_refit::RefitCancellationRequest),
    CompatibleUpdate(update::RegistryCompatibleUpdate),
    DerivedCompatibleUpdate(update::RegistryCompatibleUpdate),
    OriginalMajor(update::RegistryCompatibleUpdate),
    HouseCompatibleUpdate(update::RegistryHouseCompatibleUpdate),
    InitialFiling(RegistryInitialFiling),
    DependentInitialFiling(RegistryInitialFiling),
    HouseInitialFiling(RegistryHouseFiling),
    PurchaseFunding(crate::managed_purchase::OriginalPurchase),
    PurchaseGrant(crate::managed_purchase::PurchaseTerminal),
    PurchaseRefund(crate::managed_purchase::PurchaseTerminal),
    EntitlementFunding(crate::managed_entitlement::EntitlementPurchase),
    EntitlementGrant(crate::managed_entitlement::EntitlementTerminal),
    EntitlementRefund(crate::managed_entitlement::EntitlementTerminal),
    RenewalDraft(crate::managed_renewal::RenewalOffer),
    RenewalHolderDraft {
        offer: crate::managed_renewal::RenewalOffer,
        basis: crate::managed_paid_resale::RenewalHolderBasisV1,
    },
    RenewalHolderRetry {
        offer: crate::managed_renewal::RenewalOffer,
        basis: crate::managed_paid_resale::RenewalHolderBasisV1,
        previous_operation: CommitToken,
    },
    RenewalRetry {
        offer: crate::managed_renewal::RenewalOffer,
        previous_operation: CommitToken,
    },
    RenewalFunding(crate::managed_renewal::RenewalMoneyRequest),
    RenewalGrant(crate::managed_renewal::RenewalMoneyRequest),
    RenewalRefund(crate::managed_renewal::RenewalMoneyRequest),
    RenewalServiceClose {
        operation: CommitToken,
        reason: crate::managed_renewal::RenewalServiceCloseReason,
    },
    RenewalClose {
        operation: CommitToken,
        reason: crate::managed_renewal::UnfundedCloseReason,
    },
    HouseRenewalDraft(crate::managed_house_renewal::RenewalOffer),
    HouseRenewalHolderDraft {
        offer: crate::managed_house_renewal::RenewalOffer,
        basis: crate::managed_paid_resale::RenewalHolderBasisV1,
    },
    HouseRenewalHolderRetry {
        offer: crate::managed_house_renewal::RenewalOffer,
        basis: crate::managed_paid_resale::RenewalHolderBasisV1,
        previous_operation: CommitToken,
    },
    HouseRenewalRetry {
        offer: crate::managed_house_renewal::RenewalOffer,
        previous_operation: CommitToken,
    },
    HouseRenewalFunding(crate::managed_house_renewal::RenewalMoneyRequest),
    HouseRenewalGrant(crate::managed_house_renewal::RenewalMoneyRequest),
    HouseRenewalRefund(crate::managed_house_renewal::RenewalMoneyRequest),
    HouseRenewalServiceClose {
        operation: CommitToken,
        reason: crate::managed_house_renewal::RenewalServiceCloseReason,
    },
    HouseRenewalClose {
        operation: CommitToken,
        reason: crate::managed_house_renewal::UnfundedCloseReason,
    },
}
impl From<RegistryInitialFiling> for EconomyRequest {
    fn from(value: RegistryInitialFiling) -> Self {
        Self::InitialFiling(value)
    }
}
pub enum EconomyOutput {
    Coverage(crate::managed_coverage::CoverageOutput),
    PaidResale(crate::managed_paid_resale::PaidResaleOutput),
    HouseFunding(crate::managed_house_purchase::HouseFundingOutput),
    HouseTerminal(crate::managed_house_purchase::HouseTerminalOutput),
    CharterPending(crate::managed_charter_transfer::CharterPendingOutput),
    CharterTransfer(charter_transfer::CharterTransferOutput),
    CoreFunding(crate::core_integration::CoreFundingOutput),
    CoreTerminal(crate::core_integration::CoreTerminalOutput),
    RefitFunding(crate::core_refit::RefitFundingOutput),
    RefitTerminal(crate::core_refit::RefitTerminalOutput),
    CompatibleUpdate(update::CompatibleUpdateOutput),
    InitialFiling(InitialFilingOutput),
    PurchaseFunding(crate::managed_purchase::FundingOutput),
    PurchaseTerminal(crate::managed_purchase::TerminalOutput),
    EntitlementFunding(crate::managed_entitlement::EntitlementFundingOutput),
    EntitlementTerminal(crate::managed_entitlement::EntitlementTerminalOutput),
    RenewalOffer(crate::managed_renewal::RenewalOfferOutput),
    RenewalFunding(crate::managed_renewal::RenewalFundingOutput),
    RenewalTerminal(crate::managed_renewal::RenewalTerminalOutput),
    HouseRenewalOffer(crate::managed_house_renewal::RenewalOfferOutput),
    HouseRenewalFunding(crate::managed_house_renewal::RenewalFundingOutput),
    HouseRenewalTerminal(crate::managed_house_renewal::RenewalTerminalOutput),
}
impl EconomyOutput {
    pub fn coverage(self) -> Result<crate::managed_coverage::CoverageOutput, ParticipantError> {
        match self {
            Self::Coverage(out) => Ok(out),
            _ => Err(refused()),
        }
    }

    pub fn paid_resale(
        self,
    ) -> Result<crate::managed_paid_resale::PaidResaleOutput, ParticipantError> {
        match self {
            Self::PaidResale(out) => Ok(out),
            _ => Err(refused()),
        }
    }

    pub fn house_funding(
        self,
    ) -> Result<crate::managed_house_purchase::HouseFundingOutput, ParticipantError> {
        match self {
            Self::HouseFunding(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn house_terminal(
        self,
    ) -> Result<crate::managed_house_purchase::HouseTerminalOutput, ParticipantError> {
        match self {
            Self::HouseTerminal(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn charter_pending(
        self,
    ) -> Result<crate::managed_charter_transfer::CharterPendingOutput, ParticipantError> {
        match self {
            Self::CharterPending(out) => Ok(out),
            _ => Err(refused()),
        }
    }
    pub fn charter_transfer(
        self,
    ) -> Result<charter_transfer::CharterTransferOutput, ParticipantError> {
        match self {
            Self::CharterTransfer(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn core_funding(
        self,
    ) -> Result<crate::core_integration::CoreFundingOutput, ParticipantError> {
        match self {
            Self::CoreFunding(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn core_terminal(
        self,
    ) -> Result<crate::core_integration::CoreTerminalOutput, ParticipantError> {
        match self {
            Self::CoreTerminal(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn refit_funding(self) -> Result<crate::core_refit::RefitFundingOutput, ParticipantError> {
        match self {
            Self::RefitFunding(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn refit_terminal(
        self,
    ) -> Result<crate::core_refit::RefitTerminalOutput, ParticipantError> {
        match self {
            Self::RefitTerminal(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn compatible_update(self) -> Result<update::CompatibleUpdateOutput, ParticipantError> {
        match self {
            Self::CompatibleUpdate(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn renewal_offer(
        self,
    ) -> Result<crate::managed_renewal::RenewalOfferOutput, ParticipantError> {
        match self {
            Self::RenewalOffer(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn house_renewal_offer(
        self,
    ) -> Result<crate::managed_house_renewal::RenewalOfferOutput, ParticipantError> {
        match self {
            Self::HouseRenewalOffer(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn renewal_funding(
        self,
    ) -> Result<crate::managed_renewal::RenewalFundingOutput, ParticipantError> {
        match self {
            Self::RenewalFunding(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn house_renewal_funding(
        self,
    ) -> Result<crate::managed_house_renewal::RenewalFundingOutput, ParticipantError> {
        match self {
            Self::HouseRenewalFunding(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn renewal_terminal(
        self,
    ) -> Result<crate::managed_renewal::RenewalTerminalOutput, ParticipantError> {
        match self {
            Self::RenewalTerminal(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn house_renewal_terminal(
        self,
    ) -> Result<crate::managed_house_renewal::RenewalTerminalOutput, ParticipantError> {
        match self {
            Self::HouseRenewalTerminal(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn entitlement_funding(
        self,
    ) -> Result<crate::managed_entitlement::EntitlementFundingOutput, ParticipantError> {
        match self {
            Self::EntitlementFunding(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn entitlement_terminal(
        self,
    ) -> Result<crate::managed_entitlement::EntitlementTerminalOutput, ParticipantError> {
        match self {
            Self::EntitlementTerminal(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn initial_filing(self) -> Result<InitialFilingOutput, ParticipantError> {
        match self {
            Self::InitialFiling(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn funding(self) -> Result<crate::managed_purchase::FundingOutput, ParticipantError> {
        match self {
            Self::PurchaseFunding(output) => Ok(output),
            _ => Err(refused()),
        }
    }
    pub fn terminal(self) -> Result<crate::managed_purchase::TerminalOutput, ParticipantError> {
        match self {
            Self::PurchaseTerminal(output) => Ok(output),
            _ => Err(refused()),
        }
    }
}
pub enum EconomyPatch {
    Coverage(Box<crate::managed_coverage::CoveragePatch>),
    PaidResale(Box<crate::managed_paid_resale::PaidResalePatch>),
    CharterPending(Box<crate::managed_charter_transfer::CharterPendingPatch>),
    CharterTransfer(Box<charter_transfer::CharterTransferPatch>),
    CoreMoney(Box<crate::core_integration::CoreMoneyPatch>),
    RefitMoney(Box<crate::core_refit::RefitMoneyPatch>),
    CompatibleUpdate(Box<update::CompatibleUpdatePatch>),
    DerivedCompatibleUpdate(Box<update::CompatibleUpdatePatch>),
    OriginalMajor(Box<update::CompatibleUpdatePatch>),
    HouseCompatibleUpdate(Box<update::CompatibleUpdatePatch>),
    InitialFiling(Box<InitialFilingPatch>),
    Purchase(Box<crate::managed_purchase::PurchasePatch>),
    RenewalOffer(Box<crate::managed_renewal::install::OfferPatch>),
    RenewalMoney(Box<crate::managed_renewal::money_install::RenewalMoneyPatch>),
    HouseRenewalOffer(Box<crate::managed_house_renewal::install::OfferPatch>),
    HouseRenewalMoney(Box<crate::managed_house_renewal::money_install::RenewalMoneyPatch>),
}
impl From<InitialFilingPatch> for EconomyPatch {
    fn from(value: InitialFilingPatch) -> Self {
        Self::InitialFiling(Box::new(value))
    }
}
pub enum BoundEconomy<'a> {
    Coverage(crate::managed_coverage::BoundCoverage<'a>),
    PaidResale(crate::managed_paid_resale::BoundPaidResale<'a>),
    CharterPending(crate::managed_charter_transfer::BoundCharterPending<'a>),
    CharterTransfer(charter_transfer::BoundCharterTransfer<'a>),
    CoreMoney(crate::core_integration::BoundCoreMoney<'a>),
    RefitMoney(crate::core_refit::BoundRefitMoney<'a>),
    CompatibleUpdate(update::BoundCompatibleUpdate<'a>),
    InitialFiling(BoundInitialFiling<'a>),
    Purchase(crate::managed_purchase::BoundPurchase<'a>),
    RenewalOffer(crate::managed_renewal::install::BoundOffer<'a>),
    RenewalMoney(crate::managed_renewal::money_install::BoundRenewalMoney<'a>),
    HouseRenewalOffer(crate::managed_house_renewal::install::BoundOffer<'a>),
    HouseRenewalMoney(crate::managed_house_renewal::money_install::BoundRenewalMoney<'a>),
}
impl BoundParticipant for BoundEconomy<'_> {
    fn apply(self) {
        match self {
            Self::PaidResale(bound) => bound.apply(),
            Self::Coverage(bound) => bound.apply(),
            Self::InitialFiling(bound) => bound.apply(),
            Self::CompatibleUpdate(bound) => bound.apply(),
            Self::CharterTransfer(bound) => bound.apply(),
            Self::CharterPending(bound) => bound.apply(),
            Self::Purchase(bound) => bound.apply(),
            Self::RenewalOffer(bound) => bound.apply(),
            Self::HouseRenewalOffer(bound) => bound.apply(),
            Self::RenewalMoney(bound) => bound.apply(),
            Self::HouseRenewalMoney(bound) => bound.apply(),
            Self::CoreMoney(bound) => bound.apply(),
            Self::RefitMoney(bound) => bound.apply(),
        }
    }
}

impl InitialFilingParticipant {
    fn prepare_coverage(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_coverage::native::Request,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        let config = self.coverage.as_ref().ok_or_else(refused)?;
        let (output, patch, writes) =
            crate::managed_coverage::install::prepare(config, state, context, request)?;
        Ok(OwnerPreparation::new(
            EconomyOutput::Coverage(output),
            EconomyPatch::Coverage(patch),
            writes,
        ))
    }
    fn prepare_resale(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_paid_resale::native::Request,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        let config = self.paid_resale.as_ref().ok_or_else(refused)?;
        let (output, patch, writes) =
            crate::managed_paid_resale::install::prepare(config, state, context, request)?;
        Ok(OwnerPreparation::new(
            EconomyOutput::PaidResale(output),
            EconomyPatch::PaidResale(patch),
            writes,
        ))
    }
    fn prepare_pending(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_charter_transfer::native::Request,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        let config = self.charter_pending.as_ref().ok_or_else(refused)?;
        let (output, patch, writes) =
            crate::managed_charter_transfer::install::prepare(config, state, context, request)?;
        Ok(OwnerPreparation::new(
            EconomyOutput::CharterPending(output),
            EconomyPatch::CharterPending(patch),
            writes,
        ))
    }
    fn prepare_core(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::core_integration::money::Request,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        let config = self.core_integration.as_ref().ok_or_else(refused)?;
        let (output, patch, writes) =
            crate::core_integration::install::prepare(config, state, context, request)?;
        let output = match output {
            crate::core_integration::money::Output::Funded(output) => {
                EconomyOutput::CoreFunding(output)
            }
            crate::core_integration::money::Output::Terminal(output) => {
                EconomyOutput::CoreTerminal(output)
            }
        };
        Ok(OwnerPreparation::new(
            output,
            EconomyPatch::CoreMoney(patch),
            writes,
        ))
    }
    fn prepare_refit(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::core_refit::money::Request,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        let config = self.core_refit.as_ref().ok_or_else(refused)?;
        let (output, patch, writes) =
            crate::core_refit::install::prepare(config, state, context, request)?;
        let output = match output {
            crate::core_refit::money::Output::Funded(output) => EconomyOutput::RefitFunding(output),
            crate::core_refit::money::Output::Terminal(output) => {
                EconomyOutput::RefitTerminal(output)
            }
        };
        Ok(OwnerPreparation::new(
            output,
            EconomyPatch::RefitMoney(patch),
            writes,
        ))
    }
    fn require_renewal(
        &self,
        context: &OwnerPrepareContext<'_>,
        id: &str,
    ) -> Result<(), ParticipantError> {
        self.renewals
            .as_ref()
            .ok_or_else(refused)?
            .require(context.action(), id)
    }
    fn require_house_renewal(
        &self,
        context: &OwnerPrepareContext<'_>,
        id: &str,
    ) -> Result<(), ParticipantError> {
        self.house_renewals
            .as_ref()
            .ok_or_else(refused)?
            .require(context.action(), id)
    }
    fn recheck_filing(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        patch: &InitialFilingPatch,
    ) -> Result<(), ParticipantError> {
        if patch.clock != context.world().world_ic_now().clock {
            return Err(refused());
        }
        patch.check_local(state)?;
        let now = self.validate_for(
            state,
            context.world(),
            context.invocation().actor(),
            &patch.request,
            &patch.plan,
            patch.house.as_ref(),
        )?;
        if now != patch.validated {
            return Err(refused());
        }
        Ok(())
    }
    fn bind_filing<'a>(
        self: Arc<Self>,
        state: &'a mut EconomyStore,
        patch: InitialFilingPatch,
    ) -> Result<BoundInitialFiling<'a>, ParticipantError> {
        patch.check_local(state)?;
        let EconomyStore {
            journal,
            snapshot,
            next_transaction,
            posts_since_boot,
            tx_index,
            transactions,
            cold_keys,
            fold,
            chunk_firsts,
            chunk_first_ids,
            debit_window,
            key_filter,
            ..
        } = state;
        let InitialFilingPatch {
            journal: prepared_journal,
            bloom,
            delta,
            transaction,
            next_transaction: after_next,
            next_posts,
            chunk,
            fold: fold_patch,
            next_window,
            request,
            ..
        } = patch;
        let journal = journal
            .bind_prepared(prepared_journal)
            .map_err(|_| refused())?;
        let bloom = key_filter.bind_insert(bloom).ok_or_else(refused)?;
        Ok(BoundInitialFiling {
            journal,
            bloom,
            snapshot,
            next_transaction,
            posts_since_boot,
            tx_index,
            transactions,
            cold_keys,
            fold,
            chunk_firsts,
            chunk_first_ids,
            debit_window,
            delta,
            transaction,
            after_next,
            next_posts,
            chunk,
            fold_patch,
            next_window,
            payer: request.payer,
        })
    }
}

impl OwnerParticipant for InitialFilingParticipant {
    type State = EconomyStore;
    type Request = EconomyRequest;
    type Output = EconomyOutput;
    type Patch = EconomyPatch;
    type Bound<'a> = BoundEconomy<'a>;
    fn prepare(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: EconomyRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        match request {
            EconomyRequest::CoverageBind(request) => {
                self.prepare_coverage_bind_request(state, context, request)
            }
            EconomyRequest::CoverageTerminal(request) => {
                self.prepare_coverage_terminal_request(state, context, request)
            }
            EconomyRequest::PaidResaleAcceptance(request) => {
                self.prepare_paid_resale_acceptance_request(state, context, request)
            }
            EconomyRequest::PaidResaleTerminal(request) => {
                self.prepare_paid_resale_terminal_request(state, context, request)
            }
            EconomyRequest::CharterPendingFunding(request) => {
                self.prepare_charter_pending_funding_request(state, context, request)
            }
            EconomyRequest::CharterPendingTerminal(request) => {
                self.prepare_charter_pending_terminal_request(state, context, request)
            }
            EconomyRequest::CoreFunding(request) => {
                self.prepare_core_funding_request(state, context, request)
            }
            EconomyRequest::CoreCompletion(request) => {
                self.prepare_core_completion_request(state, context, request)
            }
            EconomyRequest::CoreCancellation(request) => {
                self.prepare_core_cancellation_request(state, context, request)
            }
            EconomyRequest::RenewalHolderDraft { offer, basis } => {
                self.prepare_renewal_holder_draft_request(state, context, offer, basis)
            }
            EconomyRequest::RenewalHolderRetry {
                offer,
                basis,
                previous_operation,
            } => self.prepare_renewal_holder_retry_request(
                state,
                context,
                offer,
                basis,
                previous_operation,
            ),
            EconomyRequest::RefitFunding(request) => {
                self.prepare_refit_funding_request(state, context, request)
            }
            EconomyRequest::RefitInstallation(request) => {
                self.prepare_refit_installation_request(state, context, request)
            }
            EconomyRequest::RefitCancellation(request) => {
                self.prepare_refit_cancellation_request(state, context, request)
            }
            EconomyRequest::RenewalDraft(request) => {
                self.prepare_renewal_draft_request(state, context, request)
            }
            EconomyRequest::RenewalRetry {
                offer,
                previous_operation,
            } => self.prepare_renewal_retry_request(state, context, offer, previous_operation),
            EconomyRequest::RenewalServiceClose { operation, reason } => {
                self.prepare_renewal_service_close_request(state, context, operation, reason)
            }
            EconomyRequest::RenewalClose { operation, reason } => {
                self.prepare_renewal_close_request(state, context, operation, reason)
            }
            EconomyRequest::RenewalFunding(request) => {
                self.prepare_renewal_funding_request(state, context, request)
            }
            EconomyRequest::RenewalGrant(request) => {
                self.prepare_renewal_grant_request(state, context, request)
            }
            EconomyRequest::RenewalRefund(request) => {
                self.prepare_renewal_refund_request(state, context, request)
            }
            EconomyRequest::HouseRenewalHolderDraft { offer, basis } => {
                self.prepare_house_renewal_holder_draft_request(state, context, offer, basis)
            }
            EconomyRequest::HouseRenewalHolderRetry {
                offer,
                basis,
                previous_operation,
            } => self.prepare_house_renewal_holder_retry_request(
                state,
                context,
                offer,
                basis,
                previous_operation,
            ),
            EconomyRequest::HouseRenewalDraft(request) => {
                self.prepare_house_renewal_draft_request(state, context, request)
            }
            EconomyRequest::HouseRenewalRetry {
                offer,
                previous_operation,
            } => {
                self.prepare_house_renewal_retry_request(state, context, offer, previous_operation)
            }
            EconomyRequest::HouseRenewalServiceClose { operation, reason } => {
                self.prepare_house_renewal_service_close_request(state, context, operation, reason)
            }
            EconomyRequest::HouseRenewalClose { operation, reason } => {
                self.prepare_house_renewal_close_request(state, context, operation, reason)
            }
            EconomyRequest::HouseRenewalFunding(request) => {
                self.prepare_house_renewal_funding_request(state, context, request)
            }
            EconomyRequest::HouseRenewalGrant(request) => {
                self.prepare_house_renewal_grant_request(state, context, request)
            }
            EconomyRequest::HouseRenewalRefund(request) => {
                self.prepare_house_renewal_refund_request(state, context, request)
            }
            EconomyRequest::CharterTransfer(request) => {
                self.prepare_charter_transfer_request(state, context, request)
            }
            EconomyRequest::OriginalMajor(request) => {
                self.prepare_original_major_request(state, context, request)
            }
            EconomyRequest::HouseCompatibleUpdate(request) => {
                self.prepare_house_compatible_update_request(state, context, request)
            }
            EconomyRequest::DerivedCompatibleUpdate(request) => {
                self.prepare_derived_update_request(state, context, request)
            }
            EconomyRequest::CompatibleUpdate(request) => {
                self.prepare_compatible_update_request(state, context, request)
            }
            EconomyRequest::InitialFiling(request) => {
                self.prepare_initial_filing_request(state, context, request)
            }
            EconomyRequest::DependentInitialFiling(request) => {
                self.prepare_dependent_initial_filing_request(state, context, request)
            }
            EconomyRequest::HouseInitialFiling(request) => {
                self.prepare_house_initial_filing_request(state, context, request)
            }
            EconomyRequest::HouseFunding(request) => {
                self.prepare_house_funding_request(state, context, request)
            }
            EconomyRequest::HouseGrant(request) => {
                self.prepare_house_grant_request(state, context, request)
            }
            EconomyRequest::HouseRefund(request) => {
                self.prepare_house_refund_request(state, context, request)
            }
            EconomyRequest::EntitlementFunding(request) => {
                self.prepare_entitlement_funding_request(state, context, request)
            }
            EconomyRequest::EntitlementGrant(request) => {
                self.prepare_entitlement_grant_request(state, context, request)
            }
            EconomyRequest::EntitlementRefund(request) => {
                self.prepare_entitlement_refund_request(state, context, request)
            }
            EconomyRequest::PurchaseFunding(request) => {
                self.prepare_purchase_funding_request(state, context, request)
            }
            EconomyRequest::PurchaseGrant(request) => {
                self.prepare_purchase_grant_request(state, context, request)
            }
            EconomyRequest::PurchaseRefund(request) => {
                self.prepare_purchase_refund_request(state, context, request)
            }
        }
    }
    fn recheck(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        patch: &EconomyPatch,
    ) -> Result<(), ParticipantError> {
        match patch {
            EconomyPatch::Coverage(patch) => {
                patch.recheck(self.coverage.as_ref().ok_or_else(refused)?, state, context)
            }
            EconomyPatch::PaidResale(patch) => patch.recheck(
                self.paid_resale.as_ref().ok_or_else(refused)?,
                state,
                context,
            ),
            EconomyPatch::CharterPending(patch) => patch.recheck(
                self.charter_pending.as_ref().ok_or_else(refused)?,
                state,
                context,
            ),
            EconomyPatch::CharterTransfer(patch) => charter_transfer::recheck(
                &self.policy,
                self.charter_transfer.as_ref().ok_or_else(refused)?,
                state,
                context,
                patch,
            ),
            EconomyPatch::CoreMoney(patch) => patch.recheck(
                self.core_integration.as_ref().ok_or_else(refused)?,
                state,
                context,
            ),
            EconomyPatch::RefitMoney(patch) => patch.recheck(
                self.core_refit.as_ref().ok_or_else(refused)?,
                state,
                context,
            ),
            EconomyPatch::HouseCompatibleUpdate(patch) => update::recheck(
                &self.policy,
                &self.house_update.as_ref().ok_or_else(refused)?.action,
                state,
                context,
                patch,
            ),
            EconomyPatch::OriginalMajor(patch) => update::recheck(
                &self.policy,
                self.original_major.as_ref().ok_or_else(refused)?,
                state,
                context,
                patch,
            ),
            EconomyPatch::DerivedCompatibleUpdate(patch) => update::recheck(
                &self.policy,
                self.derived_update.as_ref().ok_or_else(refused)?,
                state,
                context,
                patch,
            ),
            EconomyPatch::CompatibleUpdate(patch) => update::recheck(
                &self.policy,
                self.compatible_update.as_ref().ok_or_else(refused)?,
                state,
                context,
                patch,
            ),
            EconomyPatch::InitialFiling(patch) => self.recheck_filing(state, context, patch),
            EconomyPatch::Purchase(patch) => patch.recheck(state, context),
            EconomyPatch::RenewalOffer(patch) => {
                self.require_renewal(context, context.action().key().id.as_str())?;
                patch.recheck(state, context)
            }
            EconomyPatch::RenewalMoney(patch) => {
                self.require_renewal(context, context.action().key().id.as_str())?;
                patch.recheck(state, context)
            }
            EconomyPatch::HouseRenewalOffer(patch) => {
                self.require_house_renewal(context, context.action().key().id.as_str())?;
                patch.recheck(state, context)
            }
            EconomyPatch::HouseRenewalMoney(patch) => {
                self.require_house_renewal(context, context.action().key().id.as_str())?;
                patch.recheck(state, context)
            }
        }
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut EconomyStore,
        patch: EconomyPatch,
    ) -> Result<BoundEconomy<'a>, ParticipantError> {
        match patch {
            EconomyPatch::PaidResale(patch) => patch.bind(state).map(BoundEconomy::PaidResale),
            EconomyPatch::Coverage(patch) => patch.bind(state).map(BoundEconomy::Coverage),
            EconomyPatch::CharterPending(patch) => {
                patch.bind(state).map(BoundEconomy::CharterPending)
            }
            EconomyPatch::CharterTransfer(patch) => {
                charter_transfer::bind(state, *patch).map(BoundEconomy::CharterTransfer)
            }
            EconomyPatch::CoreMoney(patch) => patch.bind(state).map(BoundEconomy::CoreMoney),
            EconomyPatch::RefitMoney(patch) => patch.bind(state).map(BoundEconomy::RefitMoney),
            EconomyPatch::CompatibleUpdate(patch)
            | EconomyPatch::DerivedCompatibleUpdate(patch)
            | EconomyPatch::OriginalMajor(patch)
            | EconomyPatch::HouseCompatibleUpdate(patch) => {
                update::bind(state, *patch).map(BoundEconomy::CompatibleUpdate)
            }
            EconomyPatch::InitialFiling(patch) => self
                .bind_filing(state, *patch)
                .map(BoundEconomy::InitialFiling),
            EconomyPatch::Purchase(patch) => patch.bind(state).map(BoundEconomy::Purchase),
            EconomyPatch::RenewalOffer(patch) => patch.bind(state).map(BoundEconomy::RenewalOffer),
            EconomyPatch::RenewalMoney(patch) => patch.bind(state).map(BoundEconomy::RenewalMoney),
            EconomyPatch::HouseRenewalOffer(patch) => {
                patch.bind(state).map(BoundEconomy::HouseRenewalOffer)
            }
            EconomyPatch::HouseRenewalMoney(patch) => {
                patch.bind(state).map(BoundEconomy::HouseRenewalMoney)
            }
        }
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        match write {
            ReplayWrite::LedgerChunkSet { write } => write.log_id() == JOURNAL_LOG,
            ReplayWrite::ModuleJsonSet { key, .. } => {
                if let Some(operation) = key
                    .as_str()
                    .strip_prefix(crate::managed_renewal::service::PREFIX)
                {
                    return self
                        .renewals
                        .as_ref()
                        .is_some_and(|c| c.service_close.is_some())
                        && operation.len() == 32
                        && operation
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                }
                if let Some(operation) = key.as_str().strip_prefix("renewal-offer:") {
                    return self.renewals.is_some()
                        && operation.len() == 32
                        && operation
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                }
                if let Some(operation) = key
                    .as_str()
                    .strip_prefix(crate::managed_house_renewal::service::PREFIX)
                {
                    return self
                        .house_renewals
                        .as_ref()
                        .is_some_and(|c| c.service_close.is_some())
                        && operation.len() == 32
                        && operation
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                }
                if let Some(operation) = key.as_str().strip_prefix("house-renewal-offer:") {
                    return self.house_renewals.is_some()
                        && operation.len() == 32
                        && operation
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                }
                if key.as_str() == crate::api::META_KEY {
                    return true;
                }
                if let Some(id) = key
                    .as_str()
                    .strip_prefix("hold:")
                    .or_else(|| key.as_str().strip_prefix("contract:"))
                {
                    return canonical_u64(id);
                }
                if let Some(id) = key.as_str().strip_prefix("account:") {
                    return canonical_u64(id);
                }
                if let Some(id) = key.as_str().strip_prefix("asset:") {
                    return id.parse::<u32>().is_ok_and(|n| n.to_string() == id);
                }
                key.as_str()
                    .strip_prefix(crate::keys::IDEM_PREFIX)
                    .is_some_and(|hash| {
                        hash.len() == 64
                            && hash
                                .bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    })
            }
            _ => false,
        }
    }
}

fn canonical_u64(value: &str) -> bool {
    value.parse::<u64>().is_ok_and(|n| n.to_string() == value)
}

impl InitialFilingPatch {
    fn check_local(&self, state: &EconomyStore) -> Result<(), ParticipantError> {
        if state.frozen
            || state.next_transaction != self.transaction.id.0
            || !self.before_meta.matches(state)
            || state.posts_since_boot != self.before_posts
            || state.snapshot.asset(self.request.shell).is_some()
            || state.snapshot.managed_titles.contains(self.request.shell)
            || !state
                .fold
                .charter_transfer_receipts
                .birth_available(self.request.shell)
            || state.tx_index.contains_key(&self.transaction.id)
            || state.transactions.contains_key(&self.transaction.id)
            || state.debit_window.get(&self.request.payer) != self.before_window.as_ref()
            || self
                .before_accounts
                .iter()
                .any(|a| state.snapshot.account(a.id) != Some(a))
            || self
                .before_instruments
                .iter()
                .any(|i| state.snapshot.instrument(&i.id) != Some(i))
            || !state
                .fold
                .filing_receipts
                .may_add(self.fold.receipt.commit_token())
        {
            return Err(refused());
        }
        Ok(())
    }
}

pub struct BoundInitialFiling<'a> {
    journal: BoundLedgerAppend<'a>,
    bloom: BoundBloomInsert<'a>,
    snapshot: &'a mut EconomySnapshot,
    next_transaction: &'a mut u64,
    posts_since_boot: &'a mut u64,
    tx_index: &'a mut BTreeMap<TransactionId, TxRef>,
    transactions: &'a mut BTreeMap<TransactionId, EconomicTransaction>,
    cold_keys: &'a mut BTreeMap<IdempotencyKey, ColdKey>,
    fold: &'a mut FoldState,
    chunk_firsts: &'a mut BTreeMap<u64, u64>,
    chunk_first_ids: &'a mut BTreeMap<u64, TransactionId>,
    debit_window: &'a mut BTreeMap<AccountId, WindowTail>,
    delta: Delta,
    transaction: EconomicTransaction,
    after_next: u64,
    next_posts: u64,
    chunk: u64,
    fold_patch: Box<InitialFold>,
    next_window: Option<WindowTail>,
    payer: AccountId,
}
impl BoundParticipant for BoundInitialFiling<'_> {
    fn apply(self) {
        let Self {
            journal,
            bloom,
            snapshot,
            next_transaction,
            posts_since_boot,
            tx_index,
            transactions,
            cold_keys,
            fold,
            chunk_firsts,
            chunk_first_ids,
            debit_window,
            delta,
            transaction,
            after_next,
            next_posts,
            chunk,
            fold_patch,
            next_window,
            payer,
        } = self;
        let _already_persisted_write = journal.apply();
        delta.apply_to(snapshot);
        let (object, birth) = fold_patch.receipt.managed_title_birth();
        snapshot.managed_titles.install_prepared(object, birth);
        *next_transaction = after_next;
        *posts_since_boot = next_posts;
        chunk_firsts.entry(chunk).or_insert(transaction.journal_seq);
        chunk_first_ids.entry(chunk).or_insert(transaction.id);
        tx_index.insert(
            transaction.id,
            TxRef {
                seq: transaction.journal_seq,
                idempotency: transaction.idempotency.clone(),
                posted_at: transaction.posted_at,
            },
        );
        cold_keys.remove(&transaction.idempotency);
        bloom.apply();
        (*fold_patch).apply(fold);
        match next_window {
            Some(window) => {
                debit_window.insert(payer, window);
            }
            None => {
                debit_window.remove(&payer);
            }
        }
        transactions.insert(transaction.id, transaction);
        // The one authoritative union already contains every durable image.
        // Deliberately no legacy EconomyObserver dispatch or Outbox publication.
    }
}
