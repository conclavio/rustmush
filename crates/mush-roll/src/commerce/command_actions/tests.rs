#![cfg(test)]

use super::*;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// 17g C2a: each method returns exactly Roll's own registered action under
/// its private id and version 1. A missing action, another owner's same id
/// (Market's `fork_paid_mind`), another kind or another version is `None`.
#[test]
fn command_actions_capture_only_registered_actions() -> TestResult {
    let mut builder = DefinitionRegistryBuilder::new();
    let mut add = |owner: &str,
                   kind: &str,
                   id: &str,
                   version: u32|
     -> Result<DefinitionRef, Box<dyn std::error::Error>> {
        Ok(builder.register(
            OwnerId::new(owner)?,
            DefinitionDescriptor::new(
                DefinitionKind::new(kind)?,
                DefinitionId::new(id)?,
                version,
                json!({"fixture":"identity only"}),
            )?,
        )?)
    };
    let reserve = add("roll", "action", RESERVE.0, 1)?;
    let original = add("roll", "action", FILE_ORIGINAL.0, 1)?;
    let update = add("roll", "action", UPDATE_ORIGINAL.0, 1)?;
    let withdraw = add("roll", "action", WITHDRAW.0, 1)?;
    // Market's same-named fork and a later Roll version are not Roll's v1.
    add("market", "action", FORK.0, 1)?;
    add("roll", "action", FORK.0, 2)?;
    // An outcome sharing the attestation's id is not the action.
    add("roll", "outcome", ATTEST.0, 1)?;
    let registry = builder.freeze()?;
    let actions = RollCommandActions::capture(&registry);
    check_eq!(actions.reserve(), Some(reserve));
    check_eq!(actions.file(false), Some(original));
    check_eq!(actions.file(true), None, "no dependent filing composed");
    check_eq!(actions.update(false), Some(update));
    check_eq!(actions.update(true), None, "no derived update composed");
    check_eq!(actions.withdraw(), Some(withdraw));
    check_eq!(actions.attest(), None, "an outcome is not the action");
    check_eq!(actions.fork(), None, "neither Market's nor a v2 fork");
    check_eq!(
        RollCommandActions::capture(&DefinitionRegistryBuilder::new().freeze()?),
        RollCommandActions::default(),
        "an empty composition captures nothing"
    );
    // The ids are the ones Roll registers.
    check_eq!(
        [
            RESERVE.0,
            FILE_ORIGINAL.0,
            FILE_DEPENDENT.0,
            UPDATE_ORIGINAL.0,
            UPDATE_DERIVED.0,
            WITHDRAW.0,
            ATTEST.0,
            ATTEST_HOUSE.0,
            FORK.0
        ],
        [
            "reserve_initial_filing_shell",
            "initial_filing",
            "dependent_initial_filing",
            "update_original_mind",
            "update_derived_mind",
            "withdraw_authored_mind",
            "attest_owner_mind",
            "attest_house_mind",
            "fork_paid_mind"
        ]
    );
    Ok(())
}

/// 17g C2d test 5: the pending charter transfer's Offer (version 2 of
/// `offer_mind_charter_transfer`; the immediate protocol's version 1 of the
/// same id is not it) and Fund, and the Registry's recall and revocation,
/// are each exactly Roll's own registered action; Market's same ids, an
/// outcome, the pending protocol's settle and refund, and an empty
/// composition capture nothing.
#[test]
fn command_actions_capture_charter_and_registry_actions() -> TestResult {
    let mut builder = DefinitionRegistryBuilder::new();
    let mut add = |owner: &str,
                   kind: &str,
                   id: &str,
                   version: u32|
     -> Result<DefinitionRef, Box<dyn std::error::Error>> {
        Ok(builder.register(
            OwnerId::new(owner)?,
            DefinitionDescriptor::new(
                DefinitionKind::new(kind)?,
                DefinitionId::new(id)?,
                version,
                json!({"fixture":"identity only"}),
            )?,
        )?)
    };
    // The immediate protocol's Offer is version 1 of the same id.
    let immediate = add("roll", "action", PENDING_OFFER.0, 1)?;
    let offer = add("roll", "action", PENDING_OFFER.0, 2)?;
    let fund = add("roll", "action", PENDING_FUND.0, 1)?;
    let recall = add("roll", "action", RECALL.0, 1)?;
    let revocation = add("roll", "action", REVOCATION.0, 1)?;
    // Decoys: another owner's ids, an outcome, a later version, and the
    // pending protocol's service-only terminals.
    add("market", "action", PENDING_FUND.0, 1)?;
    add("market", "action", RECALL.0, 1)?;
    add("roll", "outcome", REVOCATION.0, 2)?;
    add("roll", "action", RECALL.0, 2)?;
    add("roll", "action", "settle_mind_charter_transfer", 1)?;
    add("roll", "action", "refund_mind_charter_transfer", 1)?;
    let registry = builder.freeze()?;
    let actions = RollCommandActions::capture(&registry);
    check_eq!(actions.pending_charter_offer(), Some(offer.clone()));
    check!(
        actions.pending_charter_offer() != Some(immediate),
        "the immediate Offer is not the pending one"
    );
    check_eq!(actions.pending_charter_fund(), Some(fund));
    check_eq!(actions.recall(), Some(recall));
    check_eq!(actions.revocation(), Some(revocation));
    let empty = RollCommandActions::capture(&DefinitionRegistryBuilder::new().freeze()?);
    check_eq!(empty.pending_charter_offer(), None);
    check_eq!(empty.pending_charter_fund(), None);
    check_eq!(empty.recall(), None);
    check_eq!(empty.revocation(), None);
    // The ids are the ones Roll registers.
    check_eq!(
        [PENDING_OFFER.0, PENDING_FUND.0, RECALL.0, REVOCATION.0],
        [
            "offer_mind_charter_transfer",
            "fund_mind_charter_transfer",
            "registry_revision_recall",
            "registry_term_revocation"
        ]
    );
    check_eq!((PENDING_OFFER.1, PENDING_FUND.1), (2, 1));
    Ok(())
}
