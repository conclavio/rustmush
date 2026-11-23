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
    AudienceSpec, Bond, Consequence, Favors, MAX_LEGAL_TARGETS, MAX_OPERATION_PARTICIPANTS,
    MAX_SIDES, MAX_STAKES, OPERATION_NOTICE_MIN, ObjectiveKind, ObjectiveSpec, OperationDoc,
    OperationRuntime, OperationState, OperationTerms, Participation, SideSpec, UnitRef,
};
use crate::render::{
    self, Lens, NO_SUCH_FORMATION, NO_SUCH_OPERATION, Verbosity, is_staff, visible_operation,
};
use crate::settle;
use crate::store::{CampaignStore, del_keys, persist_meta, put_formation, put_operation};
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

/// The same ask, reachable from cut 10's own seat predicates (`seatcmd`). One
/// authority helper for the whole crate, not two.
pub(crate) fn authorized_for(ctx: &Ctx<'_>, capability: &str, scope: OrgScopeId) -> bool {
    authorized(ctx, capability, scope)
}

// ---- E7 cut 9 fold, C1 / decision E1(a): the command predicates -----------
//
// `render::lens_for` decides **disclosure** and says so in its own doc comment; it
// grants `Lens::Plan` to staff, to the issuer, to any named side's principal and to
// any committed unit's controller, because those are the parties entitled to *see*
// the plan. Cut 9 then used `lens != Lens::Plan` as the ENTIRE authority on
// `@op/terms`, `@op/activate`, `@op/join`, `@op/withdraw` and `@op/resolve` — so a
// named side, a bystander whose object had been joined, or the bond's beneficiary
// could rewrite the pre-activation surface, reserve the issuer's money and forfeit
// it to an account they picked themselves.
//
// The lens is unchanged and stays the ONE projection predicate. These are separate,
// local, per-verb questions beside it, each stating its own rule, and each refusing
// with the same `deny()` so nothing new leaks. Every one lives inside a handler, so
// no command class moves.

/// **Who may amend an operation's terms**: staff, or whoever acts for the issuer.
/// The freeze (`state == Declared`) is a *different* question and both apply.
fn may_amend(ctx: &Ctx<'_>, doc: &OperationDoc) -> bool {
    render::acts_for(ctx.w, ctx.actor, &doc.terms.attacker)
}

/// **Who may activate**: the same set. Activation reserves the *issuer's* money, and
/// `bond::place` posts under `terms.attacker` whoever typed the command.
fn may_activate(ctx: &Ctx<'_>, doc: &OperationDoc) -> bool {
    may_amend(ctx, doc)
}

/// **Who may move the roster**: staff or the issuer. A unit's own commander reaches
/// the roster through [`may_commit_unit`] instead, which is the C6 half.
pub(crate) fn may_roster(ctx: &Ctx<'_>, doc: &OperationDoc) -> bool {
    may_amend(ctx, doc)
}

/// **Who may commit or release one unit** (E7 cut 9 fold, C6): staff, the unit's own
/// controller, or whoever holds `force.command` over a formation that unit belongs
/// to.
///
/// Cut 9's bare-unit arm of `@op/join` was `parse_obj(&rhs)` and nothing else — no
/// existence check, no control check, no `force.*` ask — so any object at all could
/// be committed. That did two things: it locked that object out of every other
/// operation (`operation_holding` is a global one-operation index), and it promoted
/// the object's controller into `lens_for`'s participant clause, which under the old
/// gate was write authority over somebody else's operation and bond.
fn may_commit_unit(ctx: &Ctx<'_>, unit: UnitRef) -> bool {
    if is_staff(ctx.w, ctx.actor) {
        return true;
    }
    if mush_core::perm::controls(ctx.w, ctx.actor, unit.0) {
        return true;
    }
    // The unit's own compartment, when it has one: a unit that is a member of a
    // formation is commanded by whoever holds `force.command` on that formation's
    // `OrgScopeId`. Bounded on nodes visited, like every other scan in the crate.
    let scopes: Vec<OrgScopeId> = ctx
        .w
        .module_state::<CampaignStore>()
        .map(|s| {
            s.formations()
                .take(crate::store::MAX_FORMATION_NODES)
                .filter(|n| n.members.contains(&unit))
                .map(|n| n.scope)
                .collect()
        })
        .unwrap_or_default();
    scopes
        .into_iter()
        .any(|scope| authorized(ctx, "force.command", scope))
}

/// **Who may resolve**: staff for every outcome, and the issuer **only** to concede
/// (`failure`).
///
/// `success` and `partial` require staff (`Role::Royalty` / `SIM_ADMIN`) or the
/// automated tick's objective evaluation — a non-staff issuer can never self-declare
/// victory from `Activated` or `Running`, evade bond forfeiture (`bond.forfeit_to`),
/// bypass declared automated objectives, or prematurely attest linked contracts
/// (P2-MOD-3). `failure` forfeits the issuer's bond to any named beneficiary
/// (doc 50 OP.0, Q-50-57 item 5) and concedes without advancing `campaign.operation.resolved`
/// fronts or attestations. `forfeit`, `abort`, and `retry` remain staff's alone.
fn may_resolve(ctx: &Ctx<'_>, doc: &OperationDoc, outcome: &str) -> bool {
    if is_staff(ctx.w, ctx.actor) {
        return true;
    }
    if outcome.trim() != "failure" {
        return false;
    }
    render::acts_for(ctx.w, ctx.actor, &doc.terms.attacker)
}

/// The **closed** outcome vocabulary `@op/resolve` accepts (E7 cut 9 fold, B7).
///
/// Cut 9 took `ctx.args.rhs()` verbatim into the document *and* into canon's
/// immutable ledger, so the only bound on either was the caller's input line; and
/// `outcome_forfeits` is `== "forfeit"`, so `Forfeit` silently released. Matching a
/// closed set case-insensitively fixes both without widening what forfeits: one word,
/// one rule, still exactly `forfeit`.
pub const OUTCOME_VOCABULARY: [&str; 6] =
    ["success", "failure", "partial", "forfeit", "abort", "retry"];

fn parse_obj(text: &str) -> Option<ObjId> {
    let raw = text.trim();
    let raw = raw.strip_prefix('#').unwrap_or(raw);
    raw.parse::<u32>().ok().map(ObjId)
}

/// A principal in a terms field: `#<obj>` or `org:<id>` (doc 50 cut OP.0, Q-50-57 item 2).
fn parse_principal(text: &str) -> Option<PrincipalId> {
    let raw = text.trim();
    if let Some(org) = raw.strip_prefix("org:") {
        return org
            .trim()
            .parse::<u64>()
            .ok()
            .map(|n| PrincipalId::Organization(mush_org::OrgId(n)));
    }
    parse_obj(raw).map(PrincipalId::Object)
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
    // E7 cut 9 fold, B8 / decision E5(a): **what an operation costs.** Declaring
    // asked for no capability, no bond, no scope and no rate limit, and the only
    // bound was the global `MAX_OPERATIONS` 512 — so one unprivileged player
    // permanently exhausted the world's allocator (nothing removes an `op:` document)
    // and starved the tick's rotation besides. The cap is per-issuer and counts only
    // operations that are still going somewhere.
    let mine = actor_principal(ctx.actor);
    let issuer_key = crate::store::principal_tag(&mine);
    let now_real = ctx.w.now();
    let staff = is_staff(ctx.w, ctx.actor);
    if !staff {
        let live = ctx
            .w
            .module_state::<CampaignStore>()
            .map_or(0, |s| s.live_operations_for(&mine));
        if live >= crate::store::MAX_OPERATIONS_PER_ISSUER {
            return Err(CmdError::msg(format!(
                "You are already running {live} operations, which is the limit of \
                 {}. Resolve or abandon one first.",
                crate::store::MAX_OPERATIONS_PER_ISSUER
            )));
        }
        if let Some(store) = ctx.w.module_state_mut::<CampaignStore>()
            && let Err(recent) = store.check_declaration_rate(&issuer_key, now_real)
        {
            return Err(CmdError::msg(format!(
                "You have declared {recent} operations recently, which is the limit of {} per \
                 hour. Wait before declaring another.",
                crate::store::MAX_DECLARATIONS_PER_ISSUER_WINDOW
            )));
        }
    }
    let (id, evicted) = ctx.w.module_state_mut::<CampaignStore>().map_or(
        (None, Vec::new()),
        CampaignStore::mint_operation_with_pruning,
    );
    del_keys(ctx, evicted);
    let Some(id) = id else {
        return Err(CmdError::msg(
            "This server is carrying as many operations as it can hold.",
        ));
    };
    const DEFAULT_WINDOW_SECS: i64 = 24 * 3_600;
    let now = ctx.w.world_timeline().stamp_at(ctx.w.now());
    let mut open = now.clone();
    open.at = now.at.saturating_add(OPERATION_NOTICE_MIN);
    let mut close = now.clone();
    // The window runs a day from its open: it shared the notice's constant while both were
    // 24 h, and stays a day now the notice is 28 IC days (owner rule, 2026-11-04).
    close.at = open.at.saturating_add(DEFAULT_WINDOW_SECS);
    let mut terms = OperationTerms {
        operation: id,
        attacker: mine,
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
    if !staff && let Some(store) = ctx.w.module_state_mut::<CampaignStore>() {
        store.record_declaration(issuer_key, now_real);
    }
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
    if lens != Lens::Plan || !may_amend(ctx, &doc) {
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
    let field = field.trim();
    apply_field(&mut terms, field, &value)?;
    if field.starts_with("bond") {
        check_bond_accounts(ctx, &terms)?;
    }
    if field == "stake.outcome" {
        check_outcome_stakes(ctx, &terms)?;
    }
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

/// Uniform existence-hiding refusal for non-staff bond account validation (P2-MOD-20).
const BOND_ACCOUNT_REFUSED: &str = "That bond account cannot be used for this operation.";
/// Uniform existence-hiding refusal for non-staff forfeit beneficiary validation (P2-MOD-20).
const FORFEIT_ACCOUNT_REFUSED: &str = "That forfeit account cannot be used for this operation.";
/// Uniform existence-hiding refusal for non-staff outcome stake validation (P2-MOD-20).
const OUTCOME_STAKE_REFUSED: &str =
    "That contract cannot be bound as an outcome stake for this operation.";

/// Whether an organization principal is active in `mush-org` (non-organization
/// principals are trivially active here).
pub(crate) fn org_principal_active(w: &mush_core::World, principal: &PrincipalId) -> bool {
    let PrincipalId::Organization(org_id) = principal else {
        return true;
    };
    w.module_state::<mush_org::OrgStore>().is_some_and(|orgs| {
        orgs.state()
            .org(*org_id)
            .is_some_and(|org| org.status == mush_org::OrgStatus::Active)
    })
}

/// **Resolve a bond's accounts at the edit** (E7 cut 9 fold, C8; P2-MOD-20).
///
/// `apply_field` parses `<account>:<minor units>` and `#<obj>:<account>` as bare
/// `u64`s and stores them; `OperationTerms::check` never looked at them either. The
/// placement side was saved by the engine (invariants 25/26 refuse a bond on someone
/// else's account, or an unfunded one) — but the **forfeit** side was validated
/// nowhere until the capture ran, inside a `Resolving` state that had no exit at all.
/// A bond whose beneficiary account is nonexistent, closed, wrong-owner, same as the
/// bond account, or on a different instrument is refused here — with uniform
/// existence-hiding refusals for non-staff callers.
fn check_bond_accounts(ctx: &Ctx<'_>, terms: &OperationTerms) -> Result<(), CmdError> {
    let staff = is_staff(ctx.w, ctx.actor);
    match bond_accounts_refusal(ctx.w, terms, staff, !staff) {
        Some(why) => Err(CmdError::msg(why)),
        None => Ok(()),
    }
}

/// `check_bond_accounts`' question, shared with the compiled declaration
/// (`crate::declare`): why `terms`' bond accounts are refused, or `None`.
///
/// `lift_issuer_owner` lifts only the rule that the bond is on the issuer's own
/// account (for staff). `hide_existence` collapses nonexistent, inactive,
/// wrong-owner, same-as-bond, and wrong-instrument refusals into one uniform
/// message per field so non-staff callers cannot probe `mush-economy` accounts.
pub(crate) fn bond_accounts_refusal(
    w: &mush_core::World,
    terms: &OperationTerms,
    lift_issuer_owner: bool,
    hide_existence: bool,
) -> Option<&'static str> {
    let bond = terms.bond.as_ref()?;
    let Some(store) = w.module_state::<mush_economy::EconomyStore>() else {
        return Some("The economy is not available, so a bond cannot be checked.");
    };
    let snapshot = store.snapshot();
    let Some(account) = snapshot.account(bond.account) else {
        return Some(if hide_existence {
            BOND_ACCOUNT_REFUSED
        } else {
            "That bond names an account that does not exist."
        });
    };
    if !matches!(account.status, mush_economy::AccountStatus::Active) {
        return Some(if hide_existence {
            BOND_ACCOUNT_REFUSED
        } else {
            "That bond names an account that is not active."
        });
    }
    // The bond is a reservation on the ISSUER's own account, so the refusal arrives
    // here rather than at `@op/activate`.
    if account.owner != terms.attacker && !lift_issuer_owner {
        return Some(if hide_existence {
            BOND_ACCOUNT_REFUSED
        } else {
            "A bond is a reservation on the issuer's own account."
        });
    }
    if let Some((beneficiary_principal, beneficiary)) = &bond.forfeit_to {
        if *beneficiary == bond.account {
            return Some(if hide_existence {
                FORFEIT_ACCOUNT_REFUSED
            } else {
                "A bond cannot forfeit back to its own account."
            });
        }
        let Some(paid) = snapshot.account(*beneficiary) else {
            return Some(if hide_existence {
                FORFEIT_ACCOUNT_REFUSED
            } else {
                "That forfeit names an account that does not exist."
            });
        };
        if !matches!(paid.status, mush_economy::AccountStatus::Active) {
            return Some(if hide_existence {
                FORFEIT_ACCOUNT_REFUSED
            } else {
                "That forfeit names an account that is not active."
            });
        }
        if paid.owner != *beneficiary_principal || !org_principal_active(w, beneficiary_principal) {
            return Some(if hide_existence {
                FORFEIT_ACCOUNT_REFUSED
            } else {
                "That forfeit account does not belong to the named beneficiary."
            });
        }
        if paid.instrument != account.instrument {
            return Some(if hide_existence {
                FORFEIT_ACCOUNT_REFUSED
            } else {
                "That forfeit account holds a different instrument from the bond's."
            });
        }
    }
    None
}

/// **A stake may only promise what something can actually settle** (E7 cut 9 fold,
/// C9 / decision E6(a); P2-MOD-20).
///
/// `required_consumers` adds `ConsumerId::ContractOutcomes` whenever a
/// `Consequence::Outcome { contract, row }` is present, and the consumer's arm is one
/// `root_recorded` call: **nothing anywhere reads `contract` or `row`**. The real
/// coupling is `mush_economy::attest::on_canon_recorded`, which fires only for a
/// contract independently authored with an `AttestorSet::Canon` naming this kind and
/// this operation's own source. So the stake is validated where it is written: the
/// named contract must exist and must already carry that attestor set (and non-staff
/// callers receive a uniform existence-hiding refusal so contracts cannot be probed).
fn check_outcome_stakes(ctx: &Ctx<'_>, terms: &OperationTerms) -> Result<(), CmdError> {
    let hide_existence = !is_staff(ctx.w, ctx.actor);
    match outcome_stakes_refusal(ctx.w, terms, hide_existence) {
        Some(why) => Err(CmdError::msg(why)),
        None => Ok(()),
    }
}

/// `check_outcome_stakes`' question, shared with the compiled declaration
/// (`crate::declare`): why `terms`' outcome stakes are refused, or `None`.
pub(crate) fn outcome_stakes_refusal(
    w: &mush_core::World,
    terms: &OperationTerms,
    hide_existence: bool,
) -> Option<String> {
    let source = SmolStr::new(settle::canon_source(terms.operation));
    for stake in &terms.stakes {
        let Consequence::Outcome { contract, .. } = stake else {
            continue;
        };
        let Some(store) = w.module_state::<mush_economy::EconomyStore>() else {
            return Some(
                "The economy is not available, so that stake cannot be checked.".to_owned(),
            );
        };
        let Some(record) = store
            .snapshot()
            .contract(mush_economy::ContractId(*contract))
        else {
            return Some(if hide_existence {
                OUTCOME_STAKE_REFUSED.to_owned()
            } else {
                "That stake names a contract that does not exist.".to_owned()
            });
        };
        if hide_existence
            && record.issuer != terms.attacker
            && !record.consent.parties.contains(&terms.attacker)
        {
            return Some(OUTCOME_STAKE_REFUSED.to_owned());
        }
        let attests = record
            .consent
            .conditions
            .iter()
            .filter_map(|c| match c {
                mush_economy::TypedCondition::Attestation { by } => Some(by),
                _ => None,
            })
            .chain(record.consent.adjudicator.iter())
            .any(|set| match set {
                mush_economy::AttestorSet::Canon {
                    kind,
                    source: named,
                } => kind == settle::KIND_OPERATION_RESOLVED && named.as_ref() == Some(&source),
                _ => false,
            });
        if !attests {
            return Some(if hide_existence {
                OUTCOME_STAKE_REFUSED.to_owned()
            } else {
                format!(
                    "Contract #{contract} carries no `{}` attestor naming `{source}`, so this \
                     operation's resolution could never settle it.",
                    settle::KIND_OPERATION_RESOLVED
                )
            });
        }
    }
    None
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
                .ok_or_else(|| {
                    bad(
                        field,
                        "`window`, `muster:<n>`, `canon:<kind>:<source>` or                          `probe:<name>:<declarer|defenders>`",
                    )
                })?;
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
                // The account is the text after the LAST colon, so `org:<id>:<account>` parses.
                let usage = "`#<obj>:<account>` or `org:<id>:<account>`";
                let (who, account) = value.rsplit_once(':').ok_or_else(|| bad(field, usage))?;
                let who = parse_principal(who).ok_or_else(|| bad(field, usage))?;
                let account: u64 = account.trim().parse().map_err(|_| bad(field, usage))?;
                bond.forfeit_to = Some((who, AccountId(account)));
            }
        }
        "side" => {
            let usage = "`<name>:#<obj>` or `<name>:org:<id>`";
            let (name, who) = value.split_once(':').ok_or_else(|| bad(field, usage))?;
            let who = parse_principal(who).ok_or_else(|| bad(field, usage))?;
            if terms.sides.len() >= MAX_SIDES {
                return Err(bad(field, "at most 8 sides"));
            }
            terms.sides.push(SideSpec {
                name: SmolStr::new(name.trim()),
                principal: who,
            });
        }
        "sides" if value == "none" => terms.sides.clear(),
        "defender" => {
            let who = parse_principal(value).ok_or_else(|| bad(field, "`#<obj>` or `org:<id>`"))?;
            terms.eligible_defenders.members.push(who);
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
    // Doc 50 OP.0 (Q-50-57 item 3): read once at the close, from the composed probe.
    if let Some(rest) = value.strip_prefix("probe:") {
        let (probe, favors) = rest.rsplit_once(':')?;
        let met_favors = match favors.trim() {
            "declarer" => Favors::Declarer,
            "defenders" => Favors::Defenders,
            _ => return None,
        };
        return Some(ObjectiveKind::ProbeAtClose {
            probe: SmolStr::new(probe.trim()),
            met_favors,
        });
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
    if lens != Lens::Plan || !may_activate(ctx, &doc) {
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
    // Doc 45 cut AP.1 (Q-45-15, Q-45-17): the notice is given when the terms freeze, which is
    // when the eligible defenders are told. The stamp moves to now and the window with it, by
    // the same amount, so the lead the issuer set is the lead a defender gets.
    let lead = notice_from_now(&mut terms, &ctx.w.world_timeline().stamp_at(ctx.w.now()));
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
    // Doc 50 §4.3 (doc 50a A-2g): the bond is placed now, so an "unfunded" pause a cut-9 tick
    // left on the declared operation no longer describes it. The tick no longer pauses a
    // declared operation at all; this clears one stored before that fix.
    if runtime.paused == Some(crate::record::PauseReason::Unfunded) {
        runtime.paused = None;
    }
    runtime.revision = runtime.revision.saturating_add(1);
    put_operation(ctx, OperationDoc { terms, runtime });
    ctx.out.info(
        ctx.actor,
        format!("Operation {id} activated; its terms are frozen."),
    );
    ctx.out.info(
        ctx.actor,
        format!(
            "Notice runs from now: the window opens in {} IC days {} hours.",
            lead.div_euclid(86_400),
            lead.rem_euclid(86_400).div_euclid(3_600)
        ),
    );
    Ok(())
}

/// **Count `terms`' notice from `now`** (doc 45 cut AP.1; `@op/activate`): the stamp moves to
/// `now` and the window by the same amount, and the hash is derived again. The lead, in IC
/// seconds. Terms stamped on another clock generation, or after `now`, are left as they are: the
/// tick pages a window on a retired clock.
fn notice_from_now(terms: &mut OperationTerms, now: &mush_core::ClockStamp) -> i64 {
    let lead = terms.window.0.at.saturating_sub(terms.notice.at);
    if terms.notice.clock != now.clock || now.at <= terms.notice.at {
        return lead;
    }
    let moved = now.at.saturating_sub(terms.notice.at);
    terms.notice.at = now.at;
    terms.window.0.at = terms.window.0.at.saturating_add(moved);
    terms.window.1.at = terms.window.1.at.saturating_add(moved);
    terms.consent_hash = operation_consent_hash(terms);
    lead
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
            // B5: a node outside the caller's authority and a node that does not
            // exist are ONE answer, so the refusal is not an existence oracle.
            return Err(deny());
        };
        // The roster question AND the node's own: the issuer moves this operation's
        // roster, and `force.command` on the node's compartment says whose formation
        // it is to deploy.
        if !may_roster(ctx, &doc) || !authorized(ctx, "force.command", node.scope) {
            return Err(deny());
        }
        (node.members.clone(), Some(formation))
    } else {
        let Some(obj) = parse_obj(&rhs) else {
            return Err(CmdError::msg("Usage: @op/join <id>=<#unit|fm<n>>"));
        };
        let unit = UnitRef(obj);
        // C6: the bare-unit arm asks about the UNIT as well as about the operation.
        // Committing an object nobody asked about locked it out of every other
        // operation and promoted its controller into the disclosure lens.
        if !may_roster(ctx, &doc) || !may_commit_unit(ctx, unit) {
            return Err(deny());
        }
        (vec![unit], None)
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
            // B3: **the refusal goes through the ONE lens too.** `operation_holding`
            // scans every document regardless of the caller's lens, and naming the
            // holder handed any player the id of every covert operation and which
            // units it held — the exact existence-hiding property `@op/show` is
            // pinned on, defeated through the verb beside it.
            let visible = ctx
                .w
                .module_state::<CampaignStore>()
                .and_then(|s| s.operation(other))
                .is_some_and(|held| render::lens_for(ctx.w, held, ctx.actor) != Lens::Hidden);
            return Err(CmdError::msg(if visible {
                format!(
                    "#{} is already committed to operation {other}; withdraw it first.",
                    unit.0.0
                )
            } else {
                format!(
                    "#{} is already committed elsewhere; withdraw it first.",
                    unit.0.0
                )
            }));
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
    let unit = UnitRef(obj);
    // C1/C6: the issuer, or the unit's own commander taking it back.
    if lens != Lens::Plan || !(may_roster(ctx, &doc) || may_commit_unit(ctx, unit)) {
        return Err(deny());
    }
    let mut runtime = doc.runtime;
    let before = runtime.participants.len();
    runtime.participants.retain(|p| p.unit != unit);
    if runtime.participants.len() == before {
        return Err(CmdError::msg("That unit is not committed to it."));
    }
    // C3: the audit tail is deduplicated and bounded on the way in, and the write
    // re-runs the constructor — ordinary play must never store a document the next
    // boot would refuse and hold, taking a live money bond out of reach.
    runtime.record_withdrawal(unit);
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
    // B7: a CLOSED vocabulary, matched case-insensitively, so the caller's bytes can
    // never be the only bound on a document — or on canon's immutable ledger.
    let raw = ctx
        .args
        .rhs()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let outcome = if raw.is_empty() {
        "success".to_owned()
    } else {
        raw
    };
    if !OUTCOME_VOCABULARY.contains(&outcome.as_str()) {
        return Err(CmdError::msg(format!(
            "Outcomes are {}.",
            OUTCOME_VOCABULARY.join(", ")
        )));
    }
    // C3: a document the boot REFUSED is held, not deleted — so the operation is not
    // in `ops`, every `@op` verb answers the existence-hiding refusal, and its bond
    // is a live reservation nothing campaign-side can reach. Staff get one path to
    // it, and it releases rather than captures.
    if is_staff(ctx.w, ctx.actor)
        && settle::outcome_abandons(&outcome)
        && ctx
            .w
            .module_state::<CampaignStore>()
            .is_some_and(|s| s.operation(id).is_none() && s.is_held(id))
    {
        return match crate::bond::release_held(ctx, id) {
            Ok(()) => {
                ctx.out.info(
                    ctx.actor,
                    format!(
                        "Operation {id}'s document is held and could not be decoded; its bond \
                         has been released. Repair or remove the document and reboot."
                    ),
                );
                Ok(())
            }
            Err(BondError::NoHold) => Err(CmdError::msg(format!(
                "Operation {id}'s document is held and it carries no live bond; repair or \
                 remove the document and reboot."
            ))),
            Err(error) => Err(CmdError::msg(format!(
                "Operation {id}'s document is held and its bond could not be released: {error}"
            ))),
        };
    }
    let Some((doc, lens)) = visible_operation(ctx.w, id, ctx.actor) else {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    };
    if lens != Lens::Plan || !may_resolve(ctx, &doc, &outcome) {
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
    if doc.runtime.state.is_terminal() {
        return Err(CmdError::msg("That operation is already resolved."));
    }
    if !settle::outcome_abandons(&outcome) && matches!(doc.runtime.state, OperationState::Declared)
    {
        return Err(CmdError::msg(
            "That operation has not been activated yet; activate it with `@op/activate` before \
             resolving.",
        ));
    }
    // **The one path past the intactness gate**, and it is staff's — now in the
    // authority as well as in the branch selection (C7), and reachable from
    // `Resolving`, which is where a stranded bond actually lives (C4).
    let entered = if settle::outcome_abandons(&outcome) {
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
    let capability = if attach {
        "force.attach"
    } else {
        "force.detach"
    };
    // B5: the node was looked up BEFORE the capability was asked about, so
    // `No such formation.` versus `Permission denied.` was an existence oracle for a
    // stranger. Both answers are now the same bytes.
    let node = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.formation(child).cloned())
        .filter(|node| authorized(ctx, capability, node.scope));
    let Some(node) = node else {
        return Err(CmdError::msg(NO_SUCH_FORMATION));
    };
    let mut node = node;
    if attach {
        let rhs = ctx.args.rhs().unwrap_or_default().trim().to_owned();
        let Some(parent) = FormationId::parse(&rhs) else {
            return Err(CmdError::msg("Usage: @force/attach <child>=<parent>"));
        };
        if parent == child {
            return Err(CmdError::msg("A formation cannot be its own parent."));
        }
        let Some(parent_node) = ctx
            .w
            .module_state::<CampaignStore>()
            .and_then(|s| s.formation(parent).cloned())
            .filter(|p| authorized(ctx, capability, p.scope))
        else {
            return Err(CmdError::msg(NO_SUCH_FORMATION));
        };
        let same_org = ctx
            .w
            .module_state::<mush_org::OrgStore>()
            .is_some_and(|orgs| {
                let st = orgs.state();
                match (st.scope(node.scope), st.scope(parent_node.scope)) {
                    (Some(c), Some(p)) => c.org == p.org,
                    _ => false,
                }
            });
        if !same_org {
            return Err(CmdError::msg(
                "A formation can only attach to a parent in the same organization.",
            ));
        }
        // B6: measure the CHILD'S OWN SUBTREE too. Cut 9 measured only the
        // parent's ancestor chain, so grafting a 3-deep chain's root under a 3-deep
        // chain's leaf produced a 6-deep tree and walked straight past the bound
        // §3.9 states as enforced. `FormationNode::check` cannot re-run it either:
        // depth is a property of the tree, not of a node.
        if depth_of(ctx, parent).saturating_add(subtree_height(ctx, child)) > MAX_FORMATION_DEPTH {
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

/// The height of the subtree rooted at `id`, in nodes, walking **downward** and
/// bounded on levels *and* on nodes visited (D-LBK-5), exactly as [`ancestors_of`] is
/// bounded upward. It stops counting one level past [`MAX_FORMATION_DEPTH`], because
/// the only question asked of it is whether a graft would exceed the cap.
fn subtree_height(ctx: &Ctx<'_>, id: FormationId) -> usize {
    let Some(store) = ctx.w.module_state::<CampaignStore>() else {
        return 1;
    };
    let mut frontier = vec![id];
    let mut height = 0usize;
    while !frontier.is_empty() && height <= MAX_FORMATION_DEPTH {
        height = height.saturating_add(1);
        let next: Vec<FormationId> = store
            .formations()
            .take(crate::store::MAX_FORMATION_NODES)
            .filter(|n| n.parent.is_some_and(|p| frontier.contains(&p)))
            .map(|n| n.id)
            .collect();
        frontier = next;
    }
    height.max(1)
}

fn force_assign(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(id) = FormationId::parse(ctx.args.lhs()) else {
        return Err(CmdError::msg(NO_SUCH_FORMATION));
    };
    let rhs = ctx.args.rhs().unwrap_or_default().trim().to_owned();
    // B5: one refusal for "no such node" and "not your node".
    let node = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.formation(id).cloned())
        .filter(|node| authorized(ctx, "force.assign", node.scope));
    let Some(node) = node else {
        return Err(CmdError::msg(NO_SUCH_FORMATION));
    };
    let (remove, raw) = match rhs.strip_prefix('-') {
        Some(rest) => (true, rest.to_owned()),
        None => (false, rhs),
    };
    let Some(obj) = parse_obj(&raw) else {
        return Err(CmdError::msg("Usage: @force/assign <id>=[-]#<unit>"));
    };
    let unit = UnitRef(obj);
    // C6: `force.assign` asked about the NODE and said nothing at all about the
    // member, so a formation was a second, quieter route to committing an object
    // nobody asked about. The member is asked about too, on both directions.
    if !may_commit_unit(ctx, unit) {
        return Err(deny());
    }
    if !remove {
        if ctx
            .w
            .get(obj)
            .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING))
        {
            return Err(CmdError::msg(
                "That unit does not exist or is being destroyed.",
            ));
        }
        // P2-MOD-60: never allow assigning an already-wrecked or quarantined
        // unit into a formation, which would retroactively turn an unapproved
        // loss into a covered-wreck recovery cooldown waiver.
        if crate::recovery::is_wreck_or_quarantined(ctx.w, obj) {
            return Err(CmdError::msg(
                "A wrecked or quarantined unit cannot be assigned to a formation.",
            ));
        }
        // P2-MOD-60: prevent hiding an unformed operational hull to obtain a
        // recovery loaner and then immediately fielding it via `@force/assign`.
        if !is_staff(ctx.w, ctx.actor)
            && crate::recovery::is_reachable_unencumbered_asset(ctx.w, obj)
            && let Some(PrincipalId::Object(owner)) = ctx
                .w
                .module_state::<mush_economy::EconomyStore>()
                .and_then(|econ| econ.snapshot().asset(obj))
                .map(|asset| asset.title_owner.clone())
        {
            let cfg = crate::recovery::policy(ctx);
            let has_live_loaner = ctx
                .w
                .module_state::<CampaignStore>()
                .and_then(|s| s.recovery_grant(owner))
                .is_some_and(|g| g.is_live() && g.loaner() != obj);
            if has_live_loaner {
                let current = crate::recovery::accessible_operational_assets(ctx.w, owner);
                let already_counted = crate::recovery::is_deployable(ctx.w, obj)
                    || ctx.w.module_state::<CampaignStore>().is_some_and(|s| {
                        s.recently_released_from_formation(obj, ctx.w.world_ic_now().at)
                    });
                let projected = if already_counted {
                    current
                } else {
                    current.saturating_add(1)
                };
                if projected > cfg.max_operational_assets {
                    return Err(CmdError::msg(
                        "Cannot assign an operational hull while its owner holds a live recovery \
                         loaner; return the loaner with `@recovery/return` first.",
                    ));
                }
            }
        }
    }
    let mut node = node;
    let was_member = node.members.contains(&unit);
    if remove {
        node.members.retain(|m| *m != unit);
    } else if !was_member {
        node.members.push(unit);
    }
    node.revision = node.revision.saturating_add(1);
    node.check()
        .map_err(|error| CmdError::msg(format!("That assignment is refused: {error}")))?;
    put_formation(ctx, node);
    // P2-MOD-60: removing a non-wrecked unit from a formation preserves its
    // recovery-eligibility accounting for `RECOVERY_COOLDOWN_SECS` so an
    // applicant cannot temporarily unassign operational hulls to evade
    // `HasAssets`. Re-assigning clears the release marker.
    if remove && was_member && !crate::recovery::is_wreck_or_quarantined(ctx.w, obj) {
        let now_ic = ctx.w.world_ic_now().at;
        if let Some(store) = ctx.w.module_state_mut::<CampaignStore>() {
            store.record_formation_release(obj, now_ic);
        }
        persist_meta(ctx);
    } else if !remove {
        let cleared = ctx
            .w
            .module_state_mut::<CampaignStore>()
            .is_some_and(|store| store.clear_formation_release(obj));
        if cleared {
            persist_meta(ctx);
        }
    }
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

/// Whether `ctx.actor` may read one formation node at all (E7 cut 9 fold, B5).
///
/// `@formation/list` and `/show` applied **no lens and no authority**: every node's
/// `OrgScopeId`, kind, roster size and parent went to any player at all — while the
/// same crate's operation surface is existence-hiding at three verbosities, and the
/// compartment id it printed is exactly the token `force.*` authority is asked about.
/// Reading is asked the same question commanding is, on the node's own scope.
fn may_read_formation(ctx: &Ctx<'_>, node: &FormationNode) -> bool {
    authorized(ctx, "force.command", node.scope)
}

fn cmd_formation(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.w.module_state::<CampaignStore>().is_none() {
        return Err(CmdError::msg(NO_SUCH_FORMATION));
    }
    if ctx.switches.has("list") {
        let nodes: Vec<FormationNode> = ctx
            .w
            .module_state::<CampaignStore>()
            .map(|s| {
                s.formations()
                    .take(crate::store::MAX_FORMATION_NODES)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let lines: Vec<String> = nodes
            .iter()
            .filter(|n| may_read_formation(ctx, n))
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
            // The same bytes a world with no formations at all answers.
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
    let node = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.formation(id).cloned())
        .filter(|node| may_read_formation(ctx, node));
    let Some(node) = node else {
        return Err(CmdError::msg(NO_SUCH_FORMATION));
    };
    let kind = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.kind(&node.kind).cloned());
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
                   activation and the bond is a reservation on your own account. The notice \
                   runs from the activation: the window moves to keep the lead you set, and \
                   each eligible defender who may see the operation is told then, by a notice \
                   that waits for their next login and by IC mail."
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
            ("notice", "720"),
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
        assert!(
            terms.check().is_err(),
            "1 hour is under the 28 IC day floor"
        );
        let mut terms = terms_fixture(OperationId(1));
        apply_field(&mut terms, "notice", "671").expect("parses");
        assert!(
            terms.check().is_err(),
            "671 hours is an hour under the 28 IC day floor"
        );
        let mut terms = terms_fixture(OperationId(1));
        apply_field(&mut terms, "notice", "2017").expect("parses");
        assert!(
            terms.check().is_err(),
            "2,017 hours is over the 84 IC day ceiling"
        );
        // Owner rule, 2026-11-04: exactly 28 IC days is the floor and is accepted.
        for hours in ["672", "720", "2016"] {
            let mut terms = terms_fixture(OperationId(1));
            apply_field(&mut terms, "notice", hours).expect("parses");
            assert!(terms.check().is_ok(), "{hours}");
        }
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
        assert_eq!(
            parse_objective("probe:isidis.tank_farm:defenders"),
            Some(ObjectiveKind::ProbeAtClose {
                probe: SmolStr::new_static("isidis.tank_farm"),
                met_favors: Favors::Defenders,
            })
        );
        assert_eq!(
            parse_objective("probe:a:b:declarer"),
            Some(ObjectiveKind::ProbeAtClose {
                probe: SmolStr::new_static("a:b"),
                met_favors: Favors::Declarer,
            })
        );
        for bad in [
            "",
            "muster",
            "muster:x",
            "canon:onlykind",
            "nope",
            "probe:onlyname",
            "probe:x:nobody",
        ] {
            assert_eq!(parse_objective(bad), None, "{bad}");
        }
    }
}
