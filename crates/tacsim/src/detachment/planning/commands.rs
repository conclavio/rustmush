//! Private planning commands share the ordinary sortie line transport.

use mush_core::{CmdError, Ctx, ObjId};
use mush_tactics::detachment::planning::{
    ConsentState, Draft, Plan, PlanHash, PlanId, PlannedController, PlanningOperation,
};

use super::{Operation, ReadRequest, ReadResponse};

pub const SWITCHES: &[&str] = &[
    "plans",
    "invitations",
    "plan",
    "preset",
    "copy",
    "load",
    "save",
    "drop-preset",
    "draft",
    "edit",
    "discard",
    "accept",
    "ready",
    "decline",
];
const MAX_INPUT_BYTES: usize = 16 * 1024;
const UNAVAILABLE: &str = "That plan or selection is unavailable. Read +sortie/plans or +sortie/invitations for current commands.";

fn fail() -> CmdError {
    CmdError::msg(UNAVAILABLE)
}

fn number(text: &str) -> Result<u64, CmdError> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(fail());
    }
    let value = text.parse().map_err(|_| fail())?;
    if value == 0 {
        return Err(fail());
    }
    Ok(value)
}

fn object(text: &str) -> Result<ObjId, CmdError> {
    u32::try_from(number(text)?).map(ObjId).map_err(|_| fail())
}

fn revision(text: &str) -> Result<(PlanId, u64), CmdError> {
    let (id, revision) = text.split_once(':').ok_or_else(fail)?;
    Ok((PlanId(number(id)?), number(revision)?))
}

fn draft_ref(text: &str, actor: ObjId) -> Result<(ObjId, PlanId), CmdError> {
    if let Some((organizer, id)) = text.split_once(':') {
        Ok((
            object(organizer.strip_prefix('#').unwrap_or(organizer))?,
            PlanId(number(id)?),
        ))
    } else {
        Ok((actor, PlanId(number(text)?)))
    }
}

fn consent_fence(text: &str) -> Result<(ObjId, PlanId, u64, PlanHash), CmdError> {
    let mut fields = text.split(':');
    let organizer = object(fields.next().ok_or_else(fail)?)?;
    let id = PlanId(number(fields.next().ok_or_else(fail)?)?);
    let revision = number(fields.next().ok_or_else(fail)?)?;
    let hash = fields.next().ok_or_else(fail)?;
    if fields.next().is_some() {
        return Err(fail());
    }
    let hash =
        serde_json::from_value(serde_json::Value::String(hash.to_owned())).map_err(|_| fail())?;
    Ok((organizer, id, revision, hash))
}

fn plan(text: &str) -> Result<Plan, CmdError> {
    if text.len() > MAX_INPUT_BYTES {
        return Err(fail());
    }
    serde_json::from_str(text).map_err(|_| fail())
}

fn line(ctx: &mut Ctx<'_>, text: impl Into<String>) {
    ctx.out
        .sys_to_actor(ctx.actor, "tacsim.detachment", text.into());
}

fn draft_summary(ctx: &mut Ctx<'_>, draft: &Draft) {
    line(
        ctx,
        format!(
            "Draft {}, revision {}, organizer #{} — +sortie/plan {}:{}",
            draft.id.0, draft.revision, draft.organizer.0, draft.organizer.0, draft.id.0
        ),
    );
}

fn show(ctx: &mut Ctx<'_>, draft: &Draft) -> Result<(), CmdError> {
    draft_summary(ctx, draft);
    line(ctx, serde_json::to_string(&draft.plan).map_err(|_| fail())?);
    line(
        ctx,
        format!(
            "Declarations: {}",
            serde_json::to_string(&draft.consents).map_err(|_| fail())?
        ),
    );
    if draft.organizer == ctx.actor {
        line(
            ctx,
            format!(
                "Edit: +sortie/edit {}:{}=<plan JSON>; save preset: +sortie/save {}:{}=<name>",
                draft.id.0, draft.revision, draft.id.0, draft.revision
            ),
        );
        line(
            ctx,
            format!("Discard: +sortie/discard {}:{}", draft.id.0, draft.revision),
        );
    }
    if draft.plan.slots.iter().any(|slot| {
        matches!(slot.controller,
        PlannedController::Player { character } if character == ctx.actor)
    }) {
        let fence = format!(
            "{}:{}:{}:{}",
            draft.organizer.0,
            draft.id.0,
            draft.revision,
            draft.plan_hash.as_str()
        );
        for verb in ["accept", "ready", "decline"] {
            line(ctx, format!("+sortie/{verb} {fence}"));
        }
        line(
            ctx,
            "Accept the assignment, then mark ready while seated. Activation will recheck current permissions and control.",
        );
    }
    super::projection::report(ctx, draft).map_err(CmdError::msg)
}

pub fn handles(ctx: &Ctx<'_>) -> bool {
    SWITCHES.iter().any(|switch| ctx.switches.has(switch))
}

pub fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.iter().count() != 1 || ctx.args.raw().len() > MAX_INPUT_BYTES {
        return Err(fail());
    }
    let raw = ctx.args.raw().trim().to_owned();
    if ctx.switches.has("plans") || ctx.switches.has("invitations") {
        if !raw.is_empty() {
            return Err(fail());
        }
        let request = if ctx.switches.has("plans") {
            ReadRequest::Own
        } else {
            ReadRequest::Invitations
        };
        match super::read(ctx, request).map_err(|_| fail())? {
            ReadResponse::Own(book) => {
                if book.presets.is_empty() && book.drafts.is_empty() {
                    line(
                        ctx,
                        "No saved plans. Use +sortie/draft <plan JSON> or +sortie/preset <name>=<plan JSON>.",
                    );
                }
                for preset in book.presets.values() {
                    line(
                        ctx,
                        format!(
                            "Preset {}: {} (revision {}) — +sortie/load {}:{}; +sortie/copy {}:{}=<new name>; +sortie/drop-preset {}:{}",
                            preset.id.0,
                            preset.name,
                            preset.revision,
                            preset.id.0,
                            preset.revision,
                            preset.id.0,
                            preset.revision,
                            preset.id.0,
                            preset.revision
                        ),
                    );
                }
                for draft in book.drafts.values() {
                    draft_summary(ctx, draft);
                }
            }
            ReadResponse::Invitations(drafts) => {
                if drafts.is_empty() {
                    line(ctx, "No current sortie invitations.");
                }
                for draft in &drafts {
                    draft_summary(ctx, draft);
                }
            }
            _ => return Err(fail()),
        }
        return Ok(());
    }
    if ctx.switches.has("plan") {
        let (organizer, id) = draft_ref(&raw, ctx.actor)?;
        let ReadResponse::Draft(draft) =
            super::read(ctx, ReadRequest::Draft { organizer, id }).map_err(|_| fail())?
        else {
            return Err(fail());
        };
        return show(ctx, &draft);
    }
    let mut organizer = ctx.actor;
    let change = if ctx.switches.has("draft") {
        PlanningOperation::CreateDraft { plan: plan(&raw)? }
    } else if ctx.switches.has("preset") {
        let (name, value) = raw.split_once('=').ok_or_else(fail)?;
        PlanningOperation::CreatePreset {
            name: name.trim().to_owned(),
            plan: plan(value.trim())?,
        }
    } else if ctx.switches.has("edit") {
        let (fence, value) = raw.split_once('=').ok_or_else(fail)?;
        let (id, expected_revision) = revision(fence.trim())?;
        PlanningOperation::EditDraft {
            id,
            expected_revision,
            plan: plan(value.trim())?,
        }
    } else if ctx.switches.has("copy") || ctx.switches.has("save") {
        let (fence, name) = raw.split_once('=').ok_or_else(fail)?;
        let (id, expected_revision) = revision(fence.trim())?;
        if ctx.switches.has("copy") {
            PlanningOperation::CopyPreset {
                id,
                expected_revision,
                name: name.trim().to_owned(),
            }
        } else {
            PlanningOperation::SaveDraftAsPreset {
                draft: id,
                expected_revision,
                name: name.trim().to_owned(),
            }
        }
    } else if ctx.switches.has("load")
        || ctx.switches.has("drop-preset")
        || ctx.switches.has("discard")
    {
        let (id, expected_revision) = revision(&raw)?;
        if ctx.switches.has("load") {
            PlanningOperation::LoadPreset {
                id,
                expected_revision,
            }
        } else if ctx.switches.has("drop-preset") {
            PlanningOperation::DeletePreset {
                id,
                expected_revision,
            }
        } else {
            PlanningOperation::DiscardDraft {
                id,
                expected_revision,
            }
        }
    } else {
        let (owner, id, expected_revision, expected_hash) = consent_fence(&raw)?;
        organizer = owner;
        let state = if ctx.switches.has("accept") {
            ConsentState::Accepted
        } else if ctx.switches.has("ready") {
            ConsentState::Ready
        } else if ctx.switches.has("decline") {
            ConsentState::Declined
        } else {
            return Err(fail());
        };
        PlanningOperation::Declare {
            id,
            expected_revision,
            expected_hash,
            state,
        }
    };
    super::submit(ctx, Operation { organizer, change }).map_err(|_| fail())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_prefills_require_every_fence_and_exact_digest() {
        let digest = "a".repeat(64);
        let (actor, id, revision, hash) =
            consent_fence(&format!("2:3:4:{digest}")).expect("valid fence");
        assert_eq!((actor, id.0, revision), (ObjId(2), 3, 4));
        assert_eq!(hash.as_str(), digest);
        for invalid in [
            format!("2:3:{digest}"),
            format!("2:3:4:{digest}:5"),
            format!("2:3:0:{digest}"),
            format!("-2:3:4:{digest}"),
            format!("2:3:18446744073709551616:{digest}"),
            format!("2:3:4:{}", "A".repeat(64)),
            "2:3:4:short".to_owned(),
        ] {
            assert!(consent_fence(&invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn edits_and_reads_do_not_accept_partial_or_negative_identities() {
        assert_eq!(revision("3:4").expect("valid revision"), (PlanId(3), 4));
        assert_eq!(
            draft_ref("3", ObjId(7)).expect("own draft"),
            (ObjId(7), PlanId(3))
        );
        assert_eq!(
            draft_ref("#8:3", ObjId(7)).expect("invited draft"),
            (ObjId(8), PlanId(3))
        );
        for invalid in ["3", "3:0", "3:4:5", "-3:4", "3:18446744073709551616"] {
            assert!(revision(invalid).is_err(), "{invalid}");
        }
        for invalid in ["0", "-3", "8:", "8:3:4"] {
            assert!(draft_ref(invalid, ObjId(7)).is_err());
        }
    }
}
