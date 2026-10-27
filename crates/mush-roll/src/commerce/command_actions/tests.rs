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
            "fork_paid_mind"
        ]
    );
    Ok(())
}
