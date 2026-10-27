//! 17g C2d (C2 plan §2.2, departure 8): the Registry operators' commands,
//! `@registry/recall <filed>` and `@registry/revoke <licence>`. They are
//! switches of `@registry` (`bulk::registry_service`), which is registered
//! only where the Registry is composed, and each wraps exactly one existing
//! Roll legal action: `roll-registry-recall` (`{"filed": "<decimal>"}`) or
//! `roll-registry-revocation` (`{"licence": "<decimal>"}`), the providers'
//! own wires; the fixed `reason` (`registry_review`) is the projection's.
//!
//! **No operator pre-check here.** Those actions are offered only to the
//! principals the deployment names and staff have granted
//! `roll.registry.recall` / `roll.registry.revoke`, so anyone else's
//! submission is refused by the server with the driver's one line, exactly
//! as an operator's naming a number that does not exist is: the command
//! answers nothing that tells a recall's target from a missing one, or an
//! operator from anyone else. A number passes through as typed (decimal,
//! nonzero). Without the command slot (a launch with no player market) or
//! with the action not composed, the command refuses `REGISTRY_OFF`, never
//! through the Registry service's own "unavailable" mapping.

use mush_core::{CmdError, Ctx};
use mush_roll::commerce::command_actions::RollCommandActions;
use serde_json::json;

use super::words::{self, fill};

/// Which Registry command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegistryCommand {
    Recall,
    Revoke,
}

fn off() -> CmdError {
    CmdError::msg(words::REGISTRY_OFF)
}

/// `@registry/recall <filed>` or `@registry/revoke <licence>`. `usage` is
/// `@registry`'s help, the reply to a malformed number.
pub(crate) fn registry(
    ctx: &mut Ctx<'_>,
    which: RegistryCommand,
    usage: &str,
) -> Result<(), CmdError> {
    let number =
        super::author::number(ctx.args.raw().trim()).ok_or_else(|| CmdError::msg(usage))?;
    super::service(ctx).map_err(|_| off())?;
    let actions = ctx
        .reg
        .definitions()
        .map(RollCommandActions::capture)
        .ok_or_else(off)?;
    let shown = number.to_string();
    let (action, verb, request, parameters) = match which {
        RegistryCommand::Recall => (
            actions.recall(),
            "recall",
            fill(words::RECALLING, &[("filed", &shown)]),
            recall_parameters(number),
        ),
        RegistryCommand::Revoke => (
            actions.revocation(),
            "revoke",
            fill(words::REVOKING, &[("licence", &shown)]),
            revocation_parameters(number),
        ),
    };
    let action = action.ok_or_else(off)?;
    super::submit(ctx, verb, &shown, action, parameters, &request)
}

/// `roll-registry-recall`'s wire: the filed revision as a decimal string.
pub(crate) fn recall_parameters(filed: u64) -> serde_json::Value {
    json!({"filed": filed.to_string()})
}

/// `roll-registry-revocation`'s wire: the licence as a decimal string.
pub(crate) fn revocation_parameters(licence: u64) -> serde_json::Value {
    json!({"licence": licence.to_string()})
}
