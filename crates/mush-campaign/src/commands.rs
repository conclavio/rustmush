//! The campaign command surface (E7 cut 9 B8).
//!
//! ## Three registration groups, each classified before its first `reg.command`
//!
//! K8.5's rule: a module MUST call `reg.classify_commands(..)` before its first
//! `reg.command(..)` or the production composition refuses. A `CommandSpec` is one
//! **primary verb**, so a verb takes the **ceiling class of its switches** — the
//! rule the tree already states for `@station`, `@journey`, `@prop` and `@domain`:
//!
//! | verb | class | why |
//! |---|---|---|
//! | `@formation` | `ReadOnly` | `/show` and `/list` read; nothing writes |
//! | `@op` | `LegacyPostImage` | the ceiling of nine switches: `/show`, `/consumers` and `/list` read, while `/declare`, `/terms`, `/activate`, `/join`, `/withdraw` and `/resolve` write the exempt `campaign.operation` document. `/activate` additionally posts an E4 `HoldOp::Place` through `mush_economy::post` — the economy's **own** carve-out lane, never `Ctx::submit_authoritative`, so the ceiling is `LegacyPostImage` and not `Authoritative` |
//! | `@force` | `LegacyPostImage` | `/create`, `/attach`, `/detach` and `/assign` write the exempt `campaign.formation` document |
//!
//! `LegacyPostImage` is restored last, so the module ends in the class its
//! documents ride.
//!
//! ## Authority
//!
//! `force.*` resolves through `mush_org::authorize_in_world` against the node's own
//! `OrgScopeId` — no core change (R2). The ids must be in
//! `org_capability_vocabulary()` or `@org/grant` refuses them loudly.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CmdError, CommandCompatibility, CommandSpec, Ctx, ObjId};
use mush_economy::{AccountId, Amount, EconomicVisibility};
use mush_org::{AuthRequest, CapabilityId, OrgScopeId, PrincipalId};
use smol_str::SmolStr;

use crate::bond::BondError;
use crate::formation::{FormationNode, MAX_FORMATION_DEPTH, Strictness};
use crate::ids::{FormationId, OperationId};
use crate::record::{
    AudienceSpec, Bond, Consequence, MAX_LEGAL_TARGETS, MAX_OPERATION_PARTICIPANTS, MAX_SIDES,
    MAX_STAKES, OPERATION_NOTICE_MIN, ObjectiveKind, ObjectiveSpec, OperationDoc, OperationRuntime,
    OperationState, OperationTerms, Participation, SideSpec, UnitRef,
};
use crate::render::{
    self, Lens, NO_SUCH_FORMATION, NO_SUCH_OPERATION, Verbosity, is_staff, visible_operation,
};
use crate::settle;
use crate::store::{CampaignStore, persist_meta, put_formation, put_operation};
use crate::terms::operation_consent_hash;

/// The `force.*` capability vocabulary this crate asks `mush-org` about. Every id
/// must be in `org_capability_vocabulary()` at the composition root, or `@org/grant`
/// fails **loudly at the command** rather than storing an inert grant.
pub const CAPABILITIES: &[&str] = &[
    "force.create",
    "force.attach",
    "force.detach",
    "force.assign",
    "force.command",
];

fn deny() -> CmdError {
    CmdError::msg("Permission denied.")
}

fn actor_principal(actor: ObjId) -> PrincipalId {
    PrincipalId::Object(actor)
}

/// Ask `mush-org` whether `actor` holds `capability` on `scope`. Staff pass, as
/// they do everywhere else in the tree.
fn authorized(ctx: &Ctx<'_>, capability: &str, scope: OrgScopeId) -> bool {
    if is_staff(ctx.w, ctx.actor) {
        return true;
    }
    let Some(capability) = CapabilityId::new(capability) else {
        return false;
    };
    mush_org::authorize_in_world(
        ctx.w,
        &AuthRequest {
            actor: ctx.actor,
            capability,
            scope,
            now: ctx.w.now(),
            amount: None,
        },
    )
    .is_allow()
}

fn parse_obj(text: &str) -> Option<ObjId> {
    let raw = text.trim();
    let raw = raw.strip_prefix('#').unwrap_or(raw);
    raw.parse::<u32>().ok().map(ObjId)
}

// ---- E7c9: B8 — @op ------------------------------------------------------

fn cmd_op(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("list") {
        return op_list(ctx);
    }
    if ctx.switches.has("declare") {
        return op_declare(ctx);
    }
    if ctx.switches.has("terms") {
        return op_terms(ctx);
    }
    if ctx.switches.has("activate") {
        return op_activate(ctx);
    }
    if ctx.switches.has("join") {
        return op_join(ctx);
    }
    if ctx.switches.has("withdraw") {
        return op_withdraw(ctx);
    }
    if ctx.switches.has("resolve") {
        return op_resolve(ctx);
    }
    if ctx.switches.has("consumers") {
        return op_consumers(ctx);
    }
    op_show(ctx)
}

fn op_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(store) = ctx.w.module_state::<CampaignStore>() else {
        ctx.out.info(ctx.actor, "No operations are declared.");
        return Ok(());
    };
    // Bounded on records VISITED (D-LBK-5), and every row goes through the ONE
    // lens: an operation a viewer may not see is not on their list at all, which
    // is the same answer as it not existing.
    let mut lines = Vec::new();
    for doc in store.operations().take(crate::store::MAX_OPERATIONS) {
        if render::lens_for(ctx.w, doc, ctx.actor) == Lens::Hidden {
            continue;
        }
        let lens = render::lens_for(ctx.w, doc, ctx.actor);
        lines.extend(render::render(doc, lens, Verbosity::Brief));
    }
    if lines.is_empty() {
        ctx.out.info(ctx.actor, "No operations are declared.");
        return Ok(());
    }
    for line in lines {
        ctx.out.info(ctx.actor, line);
    }
    Ok(())
}

fn op_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let verbosity = if ctx.switches.has("full") {
        Verbosity::Full
    } else if ctx.switches.has("brief") {
        Verbosity::Brief
    } else {
        Verbosity::Normal
    };
    let Some(id) = OperationId::parse(ctx.args.lhs()) else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    let Some((doc, lens)) = visible_operation(ctx.w, id, ctx.actor) else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    for line in render::render(&doc, lens, verbosity) {
        ctx.out.info(ctx.actor, line);
    }
    Ok(())
}

fn op_consumers(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // Staff only, and a non-staff caller gets the SAME bytes a nonexistent id
    // gets, so "you may not ask" and "there is nothing" are one answer.
    if !is_staff(ctx.w, ctx.actor) {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    }
    let Some(id) = OperationId::parse(ctx.args.lhs()) else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    let Some(doc) = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.operation(id).cloned())
    else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    let required: Vec<&str> = doc.runtime.required.iter().map(|c| c.as_str()).collect();
    let pending = settle::outstanding(&doc);
    let pending: Vec<&str> = pending.iter().map(|c| c.as_str()).collect();
    ctx.out.info(
        ctx.actor,
        format!(
            "Operation {id}: state {}, required [{}], outstanding [{}].",
            doc.runtime.state.label(),
            required.join(", "),
            pending.join(", ")
        ),
    );
    if !doc.runtime.unknown_consumers.is_empty() {
        let unknown: Vec<&str> = doc
            .runtime
            .unknown_consumers
            .iter()
            .map(SmolStr::as_str)
            .collect();
        ctx.out.info(
            ctx.actor,
            format!(
                "[staff] this build does not know consumer(s) [{}]; the operation cannot reach \
                 Resolved and has been paged.",
                unknown.join(", ")
            ),
        );
    }
    if let Some(paused) = &doc.runtime.paused {
        ctx.out
            .info(ctx.actor, format!("[staff] paused: {}.", paused.label()));
    }
    Ok(())
}

fn op_declare(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let summary = ctx.args.lhs().trim().to_owned();
    if summary.is_empty() {
        return Err(CmdError::msg("Usage: @op/declare <objective>"));
    }
    let Some(id) = ctx
        .w
        .module_state_mut::<CampaignStore>()
        .and_then(CampaignStore::mint_operation)
    else {
        return Err(CmdError::msg(
            "This server is carrying as many operations as it can hold.",
        ));
    };
    let now = ctx.w.world_timeline().stamp_at(ctx.w.now());
    let mut open = now.clone();
    open.at = now.at.saturating_add(OPERATION_NOTICE_MIN);
    let mut close = now.clone();
    close.at = open.at.saturating_add(OPERATION_NOTICE_MIN);
    let mut terms = OperationTerms {
        operation: id,
        attacker: actor_principal(ctx.actor),
        scope: None,
        objective: ObjectiveSpec {
            kind: ObjectiveKind::WindowElapsed,
            summary: SmolStr::new(&summary),
        },
        legal_targets: Vec::new(),
        stakes: Vec::new(),
        bond: None,
        sides: Vec::new(),
        eligible_defenders: AudienceSpec::default(),
        notice: now,
        window: (open, close),
        visibility: EconomicVisibility::Audience {
            members: Vec::new(),
            announce_issuer: false,
        },
        consent_hash: [0; 32],
    };
    terms.consent_hash = operation_consent_hash(&terms);
    terms
        .check()
        .map_err(|error| CmdError::msg(format!("That operation cannot be declared: {error}")))?;
    put_operation(
        ctx,
        OperationDoc {
            terms,
            runtime: OperationRuntime::declared(id),
        },
    );
    persist_meta(ctx);
    ctx.out.info(
        ctx.actor,
        format!("Operation {id} declared. Set its terms with `@op/terms {id}/<field>=<value>`."),
    );
    Ok(())
}

/// `@op/terms <id>/<field>=<value>` — **pre-activation only**. Every edit re-runs
/// the constructor and re-derives the hash, so an operation is never left holding a
/// surface it would refuse on load.
fn op_terms(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().trim().to_owned();
    let value = ctx.args.rhs().unwrap_or_default().trim().to_owned();
    let (id_text, field) = lhs
        .split_once('/')
        .ok_or_else(|| CmdError::msg("Usage: @op/terms <id>/<field>=<value>"))?;
    let Some(id) = OperationId::parse(id_text) else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    let Some((doc, lens)) = visible_operation(ctx.w, id, ctx.actor) else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    if lens != Lens::Plan {
        return Err(deny());
    }
    // **Terms freeze at activation.** Every later state refuses, and the refusal
    // says so rather than pretending the field does not exist.
    if !matches!(doc.runtime.state, OperationState::Declared) {
        return Err(CmdError::msg(
            "Those terms froze when the operation was activated; they cannot be changed.",
        ));
    }
    let mut terms = doc.terms.clone();
    apply_field(&mut terms, field.trim(), &value)?;
    terms.consent_hash = operation_consent_hash(&terms);
    terms
        .check()
        .map_err(|error| CmdError::msg(format!("Those terms are refused: {error}")))?;
    put_operation(
        ctx,
        OperationDoc {
            terms,
            runtime: doc.runtime,
        },
    );
    ctx.out
        .info(ctx.actor, format!("Operation {id}: {field} set."));
    Ok(())
}

fn bad(field: &str, usage: &str) -> CmdError {
    CmdError::msg(format!("`{field}` takes {usage}."))
}

#[allow(clippy::too_many_lines)]
fn apply_field(terms: &mut OperationTerms, field: &str, value: &str) -> Result<(), CmdError> {
    match field {
        "objective" => terms.objective.summary = SmolStr::new(value),
        "objective.kind" => {
            terms.objective.kind = parse_objective(value)
                .ok_or_else(|| bad(field, "`window`, `muster:<n>` or `canon:<kind>:<source>`"))?;
        }
        "target" => {
            if terms.legal_targets.len() >= MAX_LEGAL_TARGETS {
                return Err(bad(field, "at most 16 classes"));
            }
            terms.legal_targets.push(SmolStr::new(value));
        }
        "targets" if value == "none" => terms.legal_targets.clear(),
        "stake.front" => {
            let (region, control) = value
                .split_once(':')
                .ok_or_else(|| bad(field, "`<region>:<control>`"))?;
            let region: u64 = region
                .trim()
                .parse()
                .map_err(|_| bad(field, "`<region>:<control>`"))?;
            if terms.stakes.len() >= MAX_STAKES {
                return Err(bad(field, "at most 8 stakes"));
            }
            terms.stakes.push(Consequence::Front {
                region,
                control: SmolStr::new(control.trim()),
            });
        }
        "stake.outcome" => {
            let (contract, row) = value
                .split_once(':')
                .ok_or_else(|| bad(field, "`<contract>:<row>`"))?;
            let contract: u64 = contract
                .trim()
                .parse()
                .map_err(|_| bad(field, "`<contract>:<row>`"))?;
            if terms.stakes.len() >= MAX_STAKES {
                return Err(bad(field, "at most 8 stakes"));
            }
            terms.stakes.push(Consequence::Outcome {
                contract,
                row: SmolStr::new(row.trim()),
            });
        }
        "stakes" if value == "none" => terms.stakes.clear(),
        "bond" if value == "none" => terms.bond = None,
        "bond" => {
            let (account, amount) = value
                .split_once(':')
                .ok_or_else(|| bad(field, "`<account>:<minor units>`"))?;
            let account: u64 = account
                .trim()
                .parse()
                .map_err(|_| bad(field, "`<account>:<minor units>`"))?;
            let amount: i64 = amount
                .trim()
                .parse()
                .map_err(|_| bad(field, "`<account>:<minor units>`"))?;
            let forfeit_to = terms.bond.as_ref().and_then(|b| b.forfeit_to.clone());
            terms.bond = Some(Bond {
                account: AccountId(account),
                amount: Amount::from_minor(amount),
                forfeit_to,
                hold: None,
            });
        }
        "bond.forfeit" => {
            let bond = terms
                .bond
                .as_mut()
                .ok_or_else(|| bad(field, "a bond first"))?;
            if value == "none" {
                bond.forfeit_to = None;
            } else {
                let (who, account) = value
                    .split_once(':')
                    .ok_or_else(|| bad(field, "`#<obj>:<account>`"))?;
                let who = parse_obj(who).ok_or_else(|| bad(field, "`#<obj>:<account>`"))?;
                let account: u64 = account
                    .trim()
                    .parse()
                    .map_err(|_| bad(field, "`#<obj>:<account>`"))?;
                bond.forfeit_to = Some((PrincipalId::Object(who), AccountId(account)));
            }
        }
        "side" => {
            let (name, who) = value
                .split_once(':')
                .ok_or_else(|| bad(field, "`<name>:#<obj>`"))?;
            let who = parse_obj(who).ok_or_else(|| bad(field, "`<name>:#<obj>`"))?;
            if terms.sides.len() >= MAX_SIDES {
                return Err(bad(field, "at most 8 sides"));
            }
            terms.sides.push(SideSpec {
                name: SmolStr::new(name.trim()),
                principal: PrincipalId::Object(who),
            });
        }
        "sides" if value == "none" => terms.sides.clear(),
        "defender" => {
            let who = parse_obj(value).ok_or_else(|| bad(field, "`#<obj>`"))?;
            terms
                .eligible_defenders
                .members
                .push(PrincipalId::Object(who));
        }
        "defenders" if value == "none" => terms.eligible_defenders.members.clear(),
        "visibility" => {
            terms.visibility = match value {
                "public" => EconomicVisibility::Public,
                "parties" => EconomicVisibility::Parties,
                "staff" | "covert" => EconomicVisibility::Staff,
                "audience" | "defenders" => EconomicVisibility::Audience {
                    members: terms.eligible_defenders.members.clone(),
                    announce_issuer: false,
                },
                "announce" => EconomicVisibility::Audience {
                    members: terms.eligible_defenders.members.clone(),
                    announce_issuer: true,
                },
                _ => {
                    return Err(bad(
                        field,
                        "`public`, `parties`, `covert`, `audience` or `announce`",
                    ));
                }
            };
        }
        "notice" => {
            let hours: i64 = value.trim().parse().map_err(|_| bad(field, "`<hours>`"))?;
            let lead = hours.saturating_mul(3_600);
            let duration = terms.window.1.at.saturating_sub(terms.window.0.at).max(1);
            terms.window.0.at = terms.notice.at.saturating_add(lead);
            terms.window.1.at = terms.window.0.at.saturating_add(duration);
        }
        "window" => {
            let hours: i64 = value.trim().parse().map_err(|_| bad(field, "`<hours>`"))?;
            terms.window.1.at = terms
                .window
                .0
                .at
                .saturating_add(hours.saturating_mul(3_600).max(1));
        }
        _ => {
            return Err(CmdError::msg(
                "Fields: objective, objective.kind, target, targets, stake.front, stake.outcome, \
                 stakes, bond, bond.forfeit, side, sides, defender, defenders, visibility, \
                 notice, window.",
            ));
        }
    }
    Ok(())
}

fn parse_objective(value: &str) -> Option<ObjectiveKind> {
    if value == "window" {
        return Some(ObjectiveKind::WindowElapsed);
    }
    if let Some(n) = value.strip_prefix("muster:") {
        return n
            .trim()
            .parse()
            .ok()
            .map(|minimum| ObjectiveKind::Muster { minimum });
    }
    if let Some(rest) = value.strip_prefix("canon:") {
        let (kind, source) = rest.split_once(':')?;
        return Some(ObjectiveKind::CanonRecorded {
            kind: SmolStr::new(kind.trim()),
            source: SmolStr::new(source.trim()),
        });
    }
    None
}

/// `@op/activate <id>` — freeze the terms and place the bond.
///
/// The order matters and is the whole never-negative argument: the hold is placed
/// **first**, and only a successful placement moves the state. An attacker without
/// funds is refused by the engine's own `InsufficientAvailable` before anything is
/// written, and the operation stays `Declared`.
fn op_activate(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(id) = OperationId::parse(ctx.args.lhs()) else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    let Some((doc, lens)) = visible_operation(ctx.w, id, ctx.actor) else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    if lens != Lens::Plan {
        return Err(deny());
    }
    if !matches!(doc.runtime.state, OperationState::Declared) {
        return Err(CmdError::msg("That operation is already activated."));
    }
    if !doc.consent_intact() {
        return Err(CmdError::msg(
            "That operation's terms no longer hash; it cannot be activated.",
        ));
    }
    let mut terms = doc.terms.clone();
    if let Some(bond) = terms.bond.clone() {
        match crate::bond::place(ctx, &terms, &bond) {
            Ok(hold) => {
                if let Some(slot) = terms.bond.as_mut() {
                    slot.hold = Some(hold);
                }
            }
            Err(BondError::Refused(reason)) => {
                // The never-negative gate: `InsufficientAvailable`. The operation
                // stays `Declared` and nothing was written.
                if is_staff(ctx.w, ctx.actor) {
                    ctx.out
                        .info(ctx.actor, format!("[staff] the bond was refused: {reason}"));
                }
                return Err(CmdError::msg(
                    "That account cannot fund the bond, so the operation stays declared.",
                ));
            }
            Err(error) => {
                if is_staff(ctx.w, ctx.actor) {
                    ctx.out
                        .info(ctx.actor, format!("[staff] the bond was refused: {error}"));
                }
                return Err(CmdError::msg(
                    "That account cannot fund the bond, so the operation stays declared.",
                ));
            }
        }
    }
    let mut runtime = doc.runtime;
    runtime.state = OperationState::Activated;
    runtime.revision = runtime.revision.saturating_add(1);
    put_operation(ctx, OperationDoc { terms, runtime });
    ctx.out.info(
        ctx.actor,
        format!("Operation {id} activated; its terms are frozen."),
    );
    Ok(())
}

/// `@op/join <id>=<#unit|fm<n>>` — **a unit is in at most one operation** (R1).
fn op_join(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(id) = OperationId::parse(ctx.args.lhs()) else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    let rhs = ctx.args.rhs().unwrap_or_default().trim().to_owned();
    let Some((doc, lens)) = visible_operation(ctx.w, id, ctx.actor) else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    if lens != Lens::Plan {
        return Err(deny());
    }
    if matches!(
        doc.runtime.state,
        OperationState::Resolving { .. } | OperationState::Resolved | OperationState::Aborted
    ) {
        return Err(CmdError::msg("That operation is no longer taking units."));
    }
    // A formation joins as its whole membership, under one command parent.
    let (units, parent) = if let Some(formation) =
        FormationId::parse(&rhs).filter(|_| rhs.trim().starts_with("fm"))
    {
        let Some(node) = ctx
            .w
            .module_state::<CampaignStore>()
            .and_then(|s| s.formation(formation).cloned())
        else {
            return Err(CmdError::msg(NO_SUCH_FORMATION));
        };
        if !authorized(ctx, "force.command", node.scope) {
            return Err(deny());
        }
        (node.members.clone(), Some(formation))
    } else {
        let Some(obj) = parse_obj(&rhs) else {
            return Err(CmdError::msg("Usage: @op/join <id>=<#unit|fm<n>>"));
        };
        (vec![UnitRef(obj)], None)
    };
    let mut runtime = doc.runtime;
    let mut added = 0usize;
    for unit in units {
        if runtime.holds(unit) {
            continue;
        }
        if let Some(other) = ctx
            .w
            .module_state::<CampaignStore>()
            .and_then(|s| s.operation_holding(unit))
        {
            return Err(CmdError::msg(format!(
                "#{} is already committed to operation {other}; withdraw it first.",
                unit.0.0
            )));
        }
        if runtime.participants.len() >= MAX_OPERATION_PARTICIPANTS {
            return Err(CmdError::msg(
                "That operation is carrying as many units as it can hold.",
            ));
        }
        runtime.participants.push(Participation {
            unit,
            side: doc
                .terms
                .sides
                .first()
                .map_or_else(|| SmolStr::new_static("attack"), |s| s.name.clone()),
            command_parent: parent,
            joined_at: ctx.w.now(),
        });
        added = added.saturating_add(1);
    }
    if let Some(formation) = parent
        && !runtime.force.formations.contains(&formation)
    {
        if runtime.force.formations.len() >= crate::formation::MAX_FORMATIONS_PER_OPERATION {
            return Err(CmdError::msg(
                "That operation is carrying as many formations as it can hold.",
            ));
        }
        runtime.force.formations.push(formation);
    }
    runtime.revision = runtime.revision.saturating_add(1);
    runtime
        .check()
        .map_err(|error| CmdError::msg(format!("That roster is refused: {error}")))?;
    put_operation(
        ctx,
        OperationDoc {
            terms: doc.terms,
            runtime,
        },
    );
    ctx.out.info(
        ctx.actor,
        format!("Operation {id}: {added} unit(s) joined."),
    );
    Ok(())
}

fn op_withdraw(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(id) = OperationId::parse(ctx.args.lhs()) else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    let rhs = ctx.args.rhs().unwrap_or_default().trim().to_owned();
    let Some(obj) = parse_obj(&rhs) else {
        return Err(CmdError::msg("Usage: @op/withdraw <id>=<#unit>"));
    };
    let Some((doc, lens)) = visible_operation(ctx.w, id, ctx.actor) else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    if lens != Lens::Plan {
        return Err(deny());
    }
    let unit = UnitRef(obj);
    let mut runtime = doc.runtime;
    let before = runtime.participants.len();
    runtime.participants.retain(|p| p.unit != unit);
    if runtime.participants.len() == before {
        return Err(CmdError::msg("That unit is not committed to it."));
    }
    runtime.withdrawals.push(unit);
    runtime.revision = runtime.revision.saturating_add(1);
    put_operation(
        ctx,
        OperationDoc {
            terms: doc.terms,
            runtime,
        },
    );
    ctx.out.info(
        ctx.actor,
        format!(
            "Operation {id}: #{} withdrawn and free to join another.",
            obj.0
        ),
    );
    Ok(())
}

/// `@op/resolve <id>=<outcome>` — enter `Resolving`, or clear a pause.
fn op_resolve(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(id) = OperationId::parse(ctx.args.lhs()) else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    let outcome = ctx.args.rhs().unwrap_or_default().trim().to_owned();
    let Some((doc, lens)) = visible_operation(ctx.w, id, ctx.actor) else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    if lens != Lens::Plan {
        return Err(deny());
    }
    if outcome == "retry" {
        // Clearing a pause is a staff decision: the page said so, with this
        // command filled in.
        if !is_staff(ctx.w, ctx.actor) {
            return Err(deny());
        }
        let mut doc = doc;
        doc.runtime.paused = None;
        doc.runtime.attempts.clear();
        doc.runtime.revision = doc.runtime.revision.saturating_add(1);
        put_operation(ctx, doc);
        ctx.out.info(
            ctx.actor,
            format!("Operation {id}: the pause is cleared; the tick will try again."),
        );
        return Ok(());
    }
    let outcome = if outcome.is_empty() {
        "success".to_owned()
    } else {
        outcome
    };
    if matches!(doc.runtime.state, OperationState::Resolved) {
        return Err(CmdError::msg("That operation is already resolved."));
    }
    // **The one path past the intactness gate**, and it is staff's: abandoning a
    // corrupted operation releases its bond rather than leaving a player's money
    // reserved to a record nothing can settle.
    let entered = if outcome == "abort" && is_staff(ctx.w, ctx.actor) {
        settle::abandon(ctx, id)
    } else {
        settle::enter_resolving(ctx, id, &outcome)
    };
    if entered {
        ctx.out.info(
            ctx.actor,
            format!(
                "Operation {id} is resolving as `{outcome}`; its consumers drain on the campaign \
                 tick."
            ),
        );
        Ok(())
    } else {
        Err(CmdError::msg(
            "That operation cannot resolve right now; staff have been told why.",
        ))
    }
}

// ---- E7c9: B8 — @force and @formation ------------------------------------

fn cmd_force(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("attach") {
        return force_attach(ctx, true);
    }
    if ctx.switches.has("detach") {
        return force_attach(ctx, false);
    }
    if ctx.switches.has("assign") {
        return force_assign(ctx);
    }
    force_create(ctx)
}

fn force_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let kind = ctx.args.lhs().trim().to_owned();
    let scope: u64 = ctx
        .args
        .rhs()
        .unwrap_or_default()
        .trim()
        .parse()
        .map_err(|_| CmdError::msg("Usage: @force/create <kind>=<org scope id>"))?;
    let scope = OrgScopeId(scope);
    if !authorized(ctx, "force.create", scope) {
        return Err(deny());
    }
    // The kind must be pack-authored: unwired, the campaign layer knows none and
    // this refuses, which is the fail-closed default.
    let Some(spec) = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.kind(&kind).cloned())
    else {
        return Err(CmdError::msg("No such formation kind is authored here."));
    };
    let Some(id) = ctx
        .w
        .module_state_mut::<CampaignStore>()
        .and_then(CampaignStore::mint_formation)
    else {
        return Err(CmdError::msg(
            "This server is carrying as many formations as it can hold.",
        ));
    };
    let node = FormationNode {
        id,
        kind: SmolStr::new(&kind),
        scope,
        parent: None,
        members: Vec::new(),
        revision: 1,
    };
    node.check()
        .map_err(|error| CmdError::msg(format!("That formation is refused: {error}")))?;
    // Strictness at CREATION reports, it does not forbid: an understrength
    // formation exists and deploys (D-E7-19).
    let qualification = node.qualify(&spec, Strictness::Creation);
    put_formation(ctx, node);
    persist_meta(ctx);
    ctx.out.info(
        ctx.actor,
        format!(
            "Formation {id} created as a {kind}{}.",
            if qualification.is_clean() {
                String::new()
            } else {
                " (understrength; it still exists and still deploys)".to_owned()
            }
        ),
    );
    Ok(())
}

fn force_attach(ctx: &mut Ctx<'_>, attach: bool) -> Result<(), CmdError> {
    let Some(child) = FormationId::parse(ctx.args.lhs()) else {
        return Err(CmdError::msg(NO_SUCH_FORMATION));
    };
    let Some(node) = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.formation(child).cloned())
    else {
        return Err(CmdError::msg(NO_SUCH_FORMATION));
    };
    let capability = if attach {
        "force.attach"
    } else {
        "force.detach"
    };
    if !authorized(ctx, capability, node.scope) {
        return Err(deny());
    }
    let mut node = node;
    if attach {
        let rhs = ctx.args.rhs().unwrap_or_default().trim().to_owned();
        let Some(parent) = FormationId::parse(&rhs) else {
            return Err(CmdError::msg("Usage: @force/attach <child>=<parent>"));
        };
        if parent == child {
            return Err(CmdError::msg("A formation cannot be its own parent."));
        }
        if ctx
            .w
            .module_state::<CampaignStore>()
            .and_then(|s| s.formation(parent))
            .is_none()
        {
            return Err(CmdError::msg(NO_SUCH_FORMATION));
        }
        if depth_of(ctx, parent) >= MAX_FORMATION_DEPTH {
            return Err(CmdError::msg(format!(
                "That tree is already {MAX_FORMATION_DEPTH} deep."
            )));
        }
        if ancestors_of(ctx, parent).contains(&child) {
            return Err(CmdError::msg("That attachment would make a cycle."));
        }
        node.parent = Some(parent);
    } else {
        node.parent = None;
    }
    node.revision = node.revision.saturating_add(1);
    node.check()
        .map_err(|error| CmdError::msg(format!("That formation is refused: {error}")))?;
    put_formation(ctx, node);
    ctx.out.info(
        ctx.actor,
        format!(
            "Formation {child} {}.",
            if attach { "attached" } else { "detached" }
        ),
    );
    Ok(())
}

/// Every ancestor of `id`, walking **bounded on nodes visited** (D-LBK-5) so a
/// corrupt tree cannot spin.
fn ancestors_of(ctx: &Ctx<'_>, id: FormationId) -> Vec<FormationId> {
    let mut out = Vec::new();
    let Some(store) = ctx.w.module_state::<CampaignStore>() else {
        return out;
    };
    let mut cursor = Some(id);
    for _ in 0..=MAX_FORMATION_DEPTH {
        let Some(current) = cursor else { break };
        out.push(current);
        cursor = store.formation(current).and_then(|n| n.parent);
    }
    out
}

fn depth_of(ctx: &Ctx<'_>, id: FormationId) -> usize {
    ancestors_of(ctx, id).len()
}

fn force_assign(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(id) = FormationId::parse(ctx.args.lhs()) else {
        return Err(CmdError::msg(NO_SUCH_FORMATION));
    };
    let rhs = ctx.args.rhs().unwrap_or_default().trim().to_owned();
    let Some(node) = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.formation(id).cloned())
    else {
        return Err(CmdError::msg(NO_SUCH_FORMATION));
    };
    if !authorized(ctx, "force.assign", node.scope) {
        return Err(deny());
    }
    let (remove, raw) = match rhs.strip_prefix('-') {
        Some(rest) => (true, rest.to_owned()),
        None => (false, rhs),
    };
    let Some(obj) = parse_obj(&raw) else {
        return Err(CmdError::msg("Usage: @force/assign <id>=[-]#<unit>"));
    };
    let unit = UnitRef(obj);
    let mut node = node;
    if remove {
        node.members.retain(|m| *m != unit);
    } else if !node.members.contains(&unit) {
        node.members.push(unit);
    }
    node.revision = node.revision.saturating_add(1);
    node.check()
        .map_err(|error| CmdError::msg(format!("That assignment is refused: {error}")))?;
    put_formation(ctx, node);
    ctx.out.info(
        ctx.actor,
        format!(
            "Formation {id}: #{} {}.",
            obj.0,
            if remove { "released" } else { "assigned" }
        ),
    );
    Ok(())
}

fn cmd_formation(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(store) = ctx.w.module_state::<CampaignStore>() else {
        return Err(CmdError::msg(NO_SUCH_FORMATION));
    };
    if ctx.switches.has("list") {
        let lines: Vec<String> = store
            .formations()
            .take(crate::store::MAX_FORMATION_NODES)
            .map(|n| {
                format!(
                    "Formation {}: {} in scope {}, {} member(s).",
                    n.id,
                    n.kind,
                    n.scope.0,
                    n.members.len()
                )
            })
            .collect();
        if lines.is_empty() {
            ctx.out.info(ctx.actor, "No formations exist.");
            return Ok(());
        }
        for line in lines {
            ctx.out.info(ctx.actor, line);
        }
        return Ok(());
    }
    let Some(id) = FormationId::parse(ctx.args.lhs()) else {
        return Err(CmdError::msg(NO_SUCH_FORMATION));
    };
    let Some(node) = store.formation(id) else {
        return Err(CmdError::msg(NO_SUCH_FORMATION));
    };
    let kind = store.kind(&node.kind).cloned();
    let mut lines = vec![format!(
        "Formation {id}: {} in scope {}, {} member(s), parent {}.",
        node.kind,
        node.scope.0,
        node.members.len(),
        node.parent
            .map_or_else(|| "none".to_owned(), |p| p.to_string())
    )];
    if let Some(kind) = kind {
        // The SAME constraint set, reported under each strictness scope — never a
        // refusal to exist or to deploy.
        for scope in Strictness::ALL {
            let q = node.qualify(&kind, scope);
            lines.push(format!(
                "  {}: {}",
                scope.label(),
                if q.is_clean() {
                    "meets the composition".to_owned()
                } else if q.understrength {
                    format!(
                        "understrength ({} of {}) — it still exists and still deploys",
                        node.members.len(),
                        kind.composition.minimum
                    )
                } else if q.overstrength {
                    format!(
                        "overstrength ({} of {})",
                        node.members.len(),
                        kind.composition.maximum
                    )
                } else {
                    "missing a required role".to_owned()
                }
            ));
        }
    }
    for line in lines {
        ctx.out.info(ctx.actor, line);
    }
    Ok(())
}

/// Register the three verbs in three classified groups (K8.5).
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    // Group 1 — reads.
    reg.classify_commands(CommandCompatibility::ReadOnly);
    reg.command(
        CommandSpec {
            name: "@formation".into(),
            switches: vec!["show".into(), "list".into()],
            shape: ArgShape::EqPair,
            help: "@formation/show <id> · @formation/list — the order of battle, and how each \
                   node measures against its kind's composition at creation, at doctrine \
                   qualification and as an operation's requirement. An understrength formation \
                   still exists and still deploys."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_formation),
    )?;
    // Group 2 — the workflow documents, restored last so the module ends in the
    // class its own records ride.
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    reg.command(
        CommandSpec {
            name: "@op".into(),
            switches: vec![
                "declare".into(),
                "terms".into(),
                "activate".into(),
                "join".into(),
                "withdraw".into(),
                "show".into(),
                "brief".into(),
                "full".into(),
                "resolve".into(),
                "consumers".into(),
                "list".into(),
            ],
            shape: ArgShape::EqPair,
            help: "@op/declare <objective> · @op/terms <id>/<field>=<value> (before activation \
                   only) · @op/activate <id> · @op/join <id>=<#unit|fm<n>> · @op/withdraw \
                   <id>=<#unit> · @op/show <id> · @op/resolve <id>=<outcome> · @op/consumers \
                   <id> (staff) · @op/list — declared operations. The terms freeze at \
                   activation and the bond is a reservation on your own account."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_op),
    )?;
    reg.command(
        CommandSpec {
            name: "@force".into(),
            switches: vec![
                "create".into(),
                "attach".into(),
                "detach".into(),
                "assign".into(),
            ],
            shape: ArgShape::EqPair,
            help: "@force/create <kind>=<org scope> · @force/attach <child>=<parent> · \
                   @force/detach <child> · @force/assign <id>=[-]#<unit> — the persistent order \
                   of battle. Authority is `force.*` on the node's own org compartment."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_force),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfix::terms_fixture;

    #[test]
    fn e7c9_b8_the_capability_vocabulary_is_the_five_force_ids_and_they_are_well_formed() {
        assert_eq!(
            CAPABILITIES,
            [
                "force.create",
                "force.attach",
                "force.detach",
                "force.assign",
                "force.command"
            ]
        );
        for id in CAPABILITIES {
            assert!(CapabilityId::new(id).is_some(), "{id} is not well-formed");
        }
    }

    #[test]
    fn e7c9_b8_every_terms_field_re_runs_the_constructor_and_moves_the_hash() {
        let mut terms = terms_fixture(OperationId(1));
        terms.consent_hash = operation_consent_hash(&terms);
        let before = terms.consent_hash;
        // Each of these is a real edit to consented content.
        for (field, value) in [
            ("objective", "take the yard"),
            ("objective.kind", "muster:3"),
            ("target", "logistics"),
            ("stake.front", "7:covert"),
            ("stake.outcome", "3:partial"),
            ("visibility", "public"),
            ("notice", "48"),
        ] {
            let mut next = terms.clone();
            apply_field(&mut next, field, value).unwrap_or_else(|e| panic!("{field}: {e}"));
            next.consent_hash = operation_consent_hash(&next);
            assert!(next.check().is_ok(), "{field}");
            assert_ne!(next.consent_hash, before, "{field} did not move the hash");
        }
        // …and an unknown field is a refusal that lists the real ones.
        assert!(apply_field(&mut terms, "nonesuch", "x").is_err());
    }

    #[test]
    fn e7c9_b8_a_notice_edit_outside_the_band_refuses_rather_than_storing() {
        let mut terms = terms_fixture(OperationId(1));
        apply_field(&mut terms, "notice", "1").expect("parses");
        assert!(terms.check().is_err(), "1 hour is under the 24 hour floor");
        let mut terms = terms_fixture(OperationId(1));
        apply_field(&mut terms, "notice", "96").expect("parses");
        assert!(
            terms.check().is_err(),
            "96 hours is over the 72 hour ceiling"
        );
        let mut terms = terms_fixture(OperationId(1));
        apply_field(&mut terms, "notice", "48").expect("parses");
        assert!(terms.check().is_ok());
    }

    #[test]
    fn e7c9_b8_the_objective_grammar_covers_every_arm_and_refuses_the_rest() {
        assert_eq!(
            parse_objective("window"),
            Some(ObjectiveKind::WindowElapsed)
        );
        assert_eq!(
            parse_objective("muster:4"),
            Some(ObjectiveKind::Muster { minimum: 4 })
        );
        assert_eq!(
            parse_objective("canon:scene.outcome:run:9#1"),
            Some(ObjectiveKind::CanonRecorded {
                kind: SmolStr::new_static("scene.outcome"),
                source: SmolStr::new_static("run:9#1"),
            })
        );
        for bad in ["", "muster", "muster:x", "canon:onlykind", "nope"] {
            assert_eq!(parse_objective(bad), None, "{bad}");
        }
    }
}
