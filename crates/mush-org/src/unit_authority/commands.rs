//! Input adapters only; every durable mutation uses the shared coordinator.
use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::{
    ArgShape, CmdError, CommandCompatibility, CommandSpec, Ctx, ModError, ObjId, Registrar,
};

use super::reader::{
    UnitAuthorityRequest, UnitAuthorityResponse, UnitGrantDraft, UnitGrantMutation,
};
use super::record::{
    RecoveryPolicy, UnitCapability, UnitGrantId, UnitGrantParent, UnitGrantee, UnitScope,
};
use super::runtime::{self, UnitAuthorityHandles};

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.classify_commands(CommandCompatibility::Authoritative);
    reg.command(CommandSpec {
        name: "@unitgrant".into(), shape: ArgShape::EqPair,
        switches: ["issue", "json", "revoke", "list", "check"].into_iter().map(Into::into).collect(),
        help: "@unitgrant/issue #unit=#character <capability> [delegate] [until=<UTC seconds>] [parent=<grant>] [return-only]\n\
               @unitgrant/json <JSON draft>  explicit organization, scope or mission-role request\n\
               @unitgrant/revoke <grant>     prospectively revoke a grant you issued\n\
               @unitgrant/list [after-id]    your issued and received grants\n\
               @unitgrant/check #unit=<capability> [delegate]\n\
               Capabilities: operate_crew, deploy, command_assigned_ai, recover_load, maintain_refit, transfer_abandon_title.\n\
               A grant conveys only its named permission. It does not move, seat, launch or order anyone."
            .into(),
        ..CommandSpec::default()
    }, Arc::new(command))?;
    Ok(())
}
fn fail() -> CmdError {
    CmdError::msg("Unit-grant request refused or unavailable.")
}
fn number(text: &str) -> Result<u64, CmdError> {
    let text = text.strip_prefix('#').unwrap_or(text);
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(fail());
    }
    text.parse().map_err(|_| fail())
}
fn object(text: &str) -> Result<ObjId, CmdError> {
    u32::try_from(number(text)?).map(ObjId).map_err(|_| fail())
}
fn capability(text: &str) -> Result<UnitCapability, CmdError> {
    match text.strip_prefix("unit.").unwrap_or(text) {
        "operate_crew" => Ok(UnitCapability::OperateCrew),
        "deploy" => Ok(UnitCapability::Deploy),
        "command_assigned_ai" => Ok(UnitCapability::CommandAssignedAi),
        "recover_load" => Ok(UnitCapability::RecoverLoad),
        "maintain_refit" => Ok(UnitCapability::MaintainRefit),
        "transfer_abandon_title" => Ok(UnitCapability::TransferAbandonTitle),
        _ => Err(fail()),
    }
}
fn issue(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let unit = object(ctx.args.lhs().trim())?;
    let mut words = ctx.args.rhs().unwrap_or("").split_whitespace();
    let grantee = object(words.next().ok_or_else(fail)?)?;
    let capability = capability(words.next().ok_or_else(fail)?)?;
    let mut draft = UnitGrantDraft {
        grantee: UnitGrantee::Character(grantee),
        capability,
        scope: UnitScope::Units {
            units: BTreeSet::from([unit]),
        },
        valid_until: None,
        delegable: false,
        recovery: RecoveryPolicy::None,
        parent: None,
    };
    let mut seen = BTreeSet::new();
    for word in words {
        let name = word.split('=').next().ok_or_else(fail)?;
        if !seen.insert(name) {
            return Err(fail());
        }
        if word == "delegate" {
            draft.delegable = true;
        } else if word == "return-only" {
            draft.recovery = RecoveryPolicy::ReturnOnly;
        } else if let Some(value) = word.strip_prefix("until=") {
            draft.valid_until = Some(value.parse().map_err(|_| fail())?);
        } else if let Some(value) = word.strip_prefix("parent=") {
            draft.parent = Some(UnitGrantParent::UnitGrant {
                id: UnitGrantId(number(value)?),
            });
        } else {
            return Err(fail());
        }
    }
    runtime::submit(ctx, UnitGrantMutation::Issue(draft)).map_err(|_| fail())
}
fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ["issue", "json", "revoke", "list", "check"]
        .iter()
        .filter(|switch| ctx.switches.has(switch))
        .count()
        != 1
    {
        return Err(fail());
    }
    if ctx.switches.has("issue") {
        return issue(ctx);
    }
    if ctx.switches.has("json") {
        let raw = ctx.args.raw();
        if raw.len() > 12 * 1024 {
            return Err(fail());
        }
        let draft = serde_json::from_str::<UnitGrantDraft>(raw).map_err(|_| fail())?;
        return runtime::submit(ctx, UnitGrantMutation::Issue(draft)).map_err(|_| fail());
    }
    if ctx.switches.has("revoke") {
        let grant = UnitGrantId(number(ctx.args.lhs().trim())?);
        if ctx.args.rhs().is_some() {
            return Err(fail());
        }
        return runtime::submit(ctx, UnitGrantMutation::Revoke { grant }).map_err(|_| fail());
    }
    let handles = ctx
        .w
        .module_state::<UnitAuthorityHandles>()
        .cloned()
        .ok_or_else(fail)?;
    if ctx.switches.has("check") {
        let unit = object(ctx.args.lhs().trim())?;
        let mut words = ctx.args.rhs().unwrap_or("").split_whitespace();
        let capability = capability(words.next().ok_or_else(fail)?)?;
        let delegate = match words.next() {
            None => false,
            Some("delegate") => true,
            _ => return Err(fail()),
        };
        if words.next().is_some() {
            return Err(fail());
        }
        let response = ctx
            .read_owner(
                &handles.reader,
                UnitAuthorityRequest::Authorize {
                    unit,
                    capability,
                    delegate,
                    mission: None,
                },
            )
            .map_err(|_| fail())?;
        let UnitAuthorityResponse::Authority { allowed } = response else {
            return Err(fail());
        };
        ctx.out.sys_to_actor(
            ctx.actor,
            "org",
            if allowed {
                "Unit permission is currently allowed."
            } else {
                "Unit permission is unavailable."
            },
        );
        return Ok(());
    }
    if ctx.switches.has("list") {
        if ctx.args.rhs().is_some() {
            return Err(fail());
        }
        let after = if ctx.args.lhs().trim().is_empty() {
            0
        } else {
            number(ctx.args.lhs().trim())?
        };
        let response = ctx
            .read_owner(&handles.reader, UnitAuthorityRequest::List { after })
            .map_err(|_| fail())?;
        let UnitAuthorityResponse::Grants { grants, more } = response else {
            return Err(fail());
        };
        if grants.is_empty() {
            ctx.out
                .sys_to_actor(ctx.actor, "org", "No visible unit grants.");
        }
        for grant in &grants {
            let line = serde_json::to_string(grant).map_err(|_| fail())?;
            ctx.out.sys_to_actor(ctx.actor, "org", line);
        }
        if more && let Some(last) = grants.last() {
            ctx.out.sys_to_actor(
                ctx.actor,
                "org",
                format!("More: @unitgrant/list {}", last.id.0),
            );
        }
        return Ok(());
    }
    Err(CmdError::msg(
        "Usage: @unitgrant/issue|json|revoke|list|check ...",
    ))
}
