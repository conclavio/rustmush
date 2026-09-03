//! The economy command surface. All money movement goes through the validated
//! [`crate::post`]/[`crate::reverse`] seam — there is no balance-set command (a
//! correction is an attributed balanced transaction).
//!
//! **E3b2 opens a bounded non-staff surface** (§4.5, D-E3-6). Creating things stays
//! staff — instruments, accounts, vaults, titles, approval rules — but spending,
//! custody, approving, and *reading* are now player verbs gated by live org
//! capability. That makes disclosure a real concern for the first time, so this
//! module carries three existence-hiding renderers ([`asset_refusal`],
//! [`vault_refusal`], [`account_refusal`], plus [`NO_SUCH_REQUEST`]): **any** refusal
//! touching a resource the actor may not *view* renders as that resource's "No such
//! …" text, so a probe cannot distinguish "denied" from "not there". Every list
//! filters by the same view standing and caps at [`LIST_CAP`]; an unauthorized or
//! nonexistent filter yields an empty list rather than an error that would confirm
//! it. The journal itself ([`econ_audit`]) stays staff-only.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::output::display_name;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ObjId, Powers, Role, World, match_object, perm,
};
use mush_org::{ApprovalPolicy, OrgScopeId, PrincipalId, SystemPrincipalId};
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::api::{self, PostOutcome};
use crate::engine::AuthEval;
use crate::ids::{AccountId, ApprovalRequestId, InstrumentId, TransactionId, VaultId};
use crate::plan::{AssetOp, CustodyOp, EconomyPlan, NamedResource, RequestedLine};
use crate::record::{
    Account, AccountStatus, Approval, ApprovalRule, ApprovalStatus, CreditPolicy, CustodyRef,
    Decision, EconomicCause, Encumbrance, IdempotencyKey, InstrumentDef, InstrumentKind,
    InstrumentStatus, NegativePolicy, Vault, VaultStatus, Verdict,
};
use crate::module::EconomyStore;

/// Register the economy commands.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "@instrument".into(),
            shape: ArgShape::EqPair,
            switches: ["register", "list"].iter().copied().map(SmolStr::new).collect(),
            help: "@instrument/register <code>=<scale>   register a currency instrument\n\
                   @instrument/list                       list instruments"
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_instrument),
    )?;
    reg.command(
        CommandSpec {
            name: "@account".into(),
            shape: ArgShape::EqPair,
            switches: ["open", "system", "approval", "close", "unfreeze", "show", "list"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@account/open <obj>=<instrument>       open a personal account (staff)\n\
                   @account/open org:<org>[/<scope>]=<instrument>\n\
                   \x20                                     open an ORG wallet on a scope (staff)\n\
                   @account/system <name>=<instrument>    register a system source/sink (staff)\n\
                   @account/approval <id>=<threshold> <policy>   high-value escalation (staff)\n\
                   \x20  policy = threshold:<n>[:<role>] | named:<n>:<obj>,<obj>… | none\n\
                   @account/close <id>                    close an empty, active account\n\
                   @account/unfreeze <id>                 clear a recovery freeze (staff)\n\
                   @account/show <id>                     inspect one account\n\
                   @account/list                          list the accounts you may see\n\
                   A threshold is a PER-TRANSACTION escalation, not a budget: two payments\n\
                   under it both post without any approval (holds are E4)."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_account),
    )?;
    reg.command(
        CommandSpec {
            name: "@econ".into(),
            shape: ArgShape::EqPair,
            switches: ["pay", "mint", "reverse", "audit", "approve", "deny", "approvals", "unfreeze"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@econ/pay <from>=<to> <amount> [asset=<obj>] [from_vault=<id>|to_vault=<id>]\n\
                   \x20                                      transfer money, and optionally the\n\
                   \x20                                      title/custody of one asset, in ONE plan\n\
                   @econ/mint <sysacct>=<to> <amount>      mint from a system source (staff)\n\
                   @econ/reverse <txid>                    reverse a transaction (staff)\n\
                   @econ/audit                             recent journal entries (staff)\n\
                   @econ/approve <id>   @econ/deny <id>    decide an approval request\n\
                   @econ/approvals                         requests awaiting your decision\n\
                   @econ/unfreeze                          clear the whole-economy freeze (staff)\n\
                   \x20                                      the latch is DURABLE: it survives reboots\n\
                   \x20                                      until it is cleared here. Account, asset and\n\
                   \x20                                      vault freezes are cleared separately.\n\
                   An approval authorizes ONE exact plan; re-run the same @econ/pay once it is\n\
                   satisfied and it posts. You can never approve your own request (four-eyes).\n\
                   A plan is single-authority in E3: the payer and the title holder must be the\n\
                   same principal (or you must be staff). Counterparty consent is E4."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_econ),
    )?;
    reg.command(
        CommandSpec {
            name: "@asset".into(),
            shape: ArgShape::EqPair,
            switches: [
                "title", "transfer", "assign", "unassign", "maintenance", "lien", "release",
                "writeoff", "unfreeze", "show", "list",
            ]
            .iter()
            .copied()
            .map(SmolStr::new)
            .collect(),
            help: "@asset/title <obj>=<principal> [scope=<id>]   create a title record (staff)\n\
                   @asset/transfer <obj>=<principal> [scope=<id>] move title (both sides authorize)\n\
                   @asset/assign <obj>=<principal> [purpose] [until=<unix>]\n\
                   @asset/unassign <obj>                         clear the assignment\n\
                   @asset/maintenance <obj>=<account|none>       point the upkeep payer\n\
                   @asset/lien <obj>=<creditor> <amount> <instrument>   record a claim (staff)\n\
                   @asset/release <obj>=<n>                      release claim number n\n\
                   @asset/writeoff <obj>=<reason>                terminal write-off\n\
                   @asset/unfreeze <obj>                         clear the recovery latch (staff)\n\
                   @asset/show <obj>   @asset/list [owner=<principal>] [scope=<id>]\n\
                   <principal> is #obj, org:<id|name>, or sys:<name>. Title never moves an object,\n\
                   and moving or @chowning an object never moves title."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_asset),
    )?;
    reg.command(
        CommandSpec {
            name: "@vault".into(),
            shape: ArgShape::EqPair,
            switches: ["create", "deposit", "withdraw", "show", "list", "freeze", "unfreeze", "close"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@vault/create <name>=container:<obj> [owner=<principal>] [scope=<id>]  (staff)\n\
                   @vault/deposit <obj>=<vault>                  put an object into a vault\n\
                   @vault/withdraw <obj>=<vault> [to=here]       take one out\n\
                   @vault/show <vault>   @vault/list\n\
                   @vault/freeze|unfreeze <vault>                staff recovery (not journaled)\n\
                   @vault/close <vault>                          close an empty vault\n\
                   Custody is not title: a deposit moves no ownership, and a deposited container\n\
                   still shows its contents."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_vault),
    )?;
    Ok(())
}

// ---- helpers --------------------------------------------------------------

fn is_admin(w: &World, who: ObjId) -> bool {
    perm::has_power(w, who, Powers::SIM_ADMIN) || perm::has_role(w, who, Role::Royalty)
}
fn require_admin(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if is_admin(ctx.w, ctx.actor) {
        Ok(())
    } else {
        Err(CmdError::msg("Permission denied."))
    }
}
/// Refuse a **non-recovery** control write while the whole economy is frozen.
///
/// A freeze is "no spending authority until staff look at it"; opening a fresh
/// wallet, re-writing an approval rule, or binding a new vault are all ways to build
/// authority on top of a state nobody has audited yet. The recovery verbs
/// (`freeze`/`unfreeze`/`close`) stay available — they are how the freeze is worked
/// through — and the read verbs never move anything.
fn refuse_if_frozen(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if ctx.w.module_state::<EconomyStore>().is_some_and(EconomyStore::is_frozen) {
        return Err(CmdError::msg(
            "The economy is frozen; that is not available until staff clear it (@econ/unfreeze).",
        ));
    }
    Ok(())
}
fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "economy", msg.into());
}
fn parse_account(s: &str) -> Option<AccountId> {
    s.trim().trim_start_matches('#').parse::<u64>().ok().map(AccountId)
}
/// A unique idempotency key for an interactive command (each invocation is a fresh
/// intent). Keyed on the next transaction id, which is unique and never reused — so
/// two identical `@econ/pay`s both post (different keys), while an ambiguously-acked
/// retry of the SAME post (same key) dedups.
fn cmd_key(ctx: &Ctx<'_>) -> IdempotencyKey {
    let serial = ctx.w.module_state::<EconomyStore>().map_or(0, EconomyStore::peek_next_transaction);
    IdempotencyKey::new(SmolStr::new_static("cmd"), SmolStr::new(serial.to_string()))
}

// ---- E3b1 shared parsing + the existence-hiding renderer ------------------

/// The row cap every `*/list` obeys (§4.5).
const LIST_CAP: usize = 20;

/// Pull `key=value` options out of a free-form tail, returning the remaining words
/// and the recognized options (lower-cased keys). Unknown `k=v` words stay in the
/// remainder, so a purpose containing an `=` is not silently eaten.
fn parse_opts(s: &str, keys: &[&str]) -> (String, BTreeMap<String, String>) {
    let mut rest: Vec<&str> = Vec::new();
    let mut opts: BTreeMap<String, String> = BTreeMap::new();
    for tok in s.split_whitespace() {
        if let Some((k, v)) = tok.split_once('=') {
            let lk = k.to_ascii_lowercase();
            if keys.contains(&lk.as_str()) {
                opts.insert(lk, v.to_string());
                continue;
            }
        }
        rest.push(tok);
    }
    (rest.join(" "), opts)
}

/// `<principal>` = `#obj` (or a matchable name) | `org:<id|name>` | `sys:<name>`.
fn parse_principal(ctx: &Ctx<'_>, s: &str) -> Option<PrincipalId> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("org:") {
        return resolve_org(ctx, rest).map(PrincipalId::Organization);
    }
    if let Some(rest) = s.strip_prefix("sys:") {
        return SystemPrincipalId::new(rest).map(PrincipalId::System);
    }
    match_object(ctx.w, ctx.actor, s).ok().map(PrincipalId::Object)
}

fn resolve_org(ctx: &Ctx<'_>, s: &str) -> Option<mush_org::OrgId> {
    let s = s.trim();
    let store = ctx.w.module_state::<mush_org::OrgStore>()?;
    if let Ok(n) = s.trim_start_matches('#').parse::<u64>() {
        let id = mush_org::OrgId(n);
        return store.state().org(id).map(|o| o.id);
    }
    let lower = s.to_ascii_lowercase();
    store
        .state()
        .orgs()
        .find(|o| {
            o.name.to_ascii_lowercase() == lower
                || o.aliases.iter().any(|a| a.to_ascii_lowercase() == lower)
        })
        .map(|o| o.id)
}

fn parse_scope(s: &str) -> Option<OrgScopeId> {
    s.trim().trim_start_matches('#').parse::<u64>().ok().map(OrgScopeId)
}
fn parse_vault_id(s: &str) -> Option<VaultId> {
    s.trim().trim_start_matches('#').parse::<u64>().ok().map(VaultId)
}

fn render_principal(ctx: &Ctx<'_>, p: &PrincipalId) -> String {
    match p {
        PrincipalId::Object(o) => format!("{} (#{})", display_name(ctx.w, *o), o.0),
        PrincipalId::Organization(o) => {
            let name = ctx
                .w
                .module_state::<mush_org::OrgStore>()
                .and_then(|s| s.state().org(*o).map(|x| x.name.to_string()));
            name.map_or_else(|| format!("org:{o}"), |n| format!("org:{n} (#{o})"))
        }
        PrincipalId::System(s) => format!("sys:{s}"),
    }
}

/// §4.5 view standing over an asset: staff, its title owner, its assignee, or a
/// holder of an `asset.*` capability on its (live) compartment.
fn may_view_asset(w: &World, actor: ObjId, object: ObjId) -> bool {
    if api::is_staff(w, actor) {
        return true;
    }
    let Some(store) = w.module_state::<EconomyStore>() else { return false };
    let Some(a) = store.snapshot().asset(object) else { return false };
    let me = PrincipalId::Object(actor);
    if a.title_owner == me || a.assignment.as_ref().is_some_and(|x| x.assigned_to == me) {
        return true;
    }
    let Some(scope) = a.scope else { return false };
    api::scope_coherent(w, Some(scope), &a.title_owner)
        && (api::holds_capability(w, actor, "asset.title.transfer", scope)
            || api::holds_capability(w, actor, "asset.assign", scope))
}

/// §4.5 view standing over an account: staff, its owner, a holder of
/// `economy.account.view` on its (live) compartment — **or** an actor named in a
/// live approval request's `NamedSet` over it. That last clause is deliberate: you
/// cannot responsibly approve a payment you are forbidden to look at, and the
/// standing lasts exactly as long as the request does.
///
/// **Public** so other modules that render an account balance render it behind the
/// SAME gate rather than inventing a second, weaker one (E3d's faction read-through
/// is the first such caller). A `false` answer means "render nothing" — never
/// "render a placeholder", which would leak the account's existence.
pub fn may_view_account(w: &World, actor: ObjId, id: AccountId) -> bool {
    if api::is_staff(w, actor) {
        return true;
    }
    let Some(store) = w.module_state::<EconomyStore>() else { return false };
    let snap = store.snapshot();
    let Some(a) = snap.account(id) else { return false };
    if a.owner == PrincipalId::Object(actor) {
        return true;
    }
    if let Some(scope) = a.scope
        && api::scope_coherent(w, Some(scope), &a.owner)
        && api::holds_capability(w, actor, "economy.account.view", scope)
    {
        return true;
    }
    let named = a
        .approval
        .as_ref()
        .is_some_and(|rule| matches!(&rule.policy, ApprovalPolicy::NamedSet { actors, .. } if actors.contains(&actor)));
    if !named {
        return false;
    }
    let now = w.now();
    snap.approvals()
        .any(|r| r.status == ApprovalStatus::Pending && now < r.expires_at && r.accounts.contains(&id))
}

/// The account half of the existence-hiding rule (§4.5): a refusal touching an
/// account the actor may not VIEW renders as the nonexistent-account text, so
/// "denied" and "there is no such account" are one message.
fn account_refusal(ctx: &Ctx<'_>, id: AccountId, detail: impl std::fmt::Display) -> CmdError {
    if may_view_account(ctx.w, ctx.actor, id) {
        CmdError::msg(detail.to_string())
    } else {
        CmdError::msg("No such account.")
    }
}

/// The "No such …" text for a resource the actor may not view, or `None` when they
/// may (and so may see the real refusal).
fn hidden_text(ctx: &Ctx<'_>, resource: NamedResource) -> Option<&'static str> {
    match resource {
        NamedResource::Account(id) => {
            (!may_view_account(ctx.w, ctx.actor, id)).then_some("No such account.")
        }
        NamedResource::Asset(o) => (!may_view_asset(ctx.w, ctx.actor, o)).then_some("No such asset."),
        NamedResource::Vault(v) => (!may_view_vault(ctx.w, ctx.actor, v)).then_some("No such vault."),
    }
}

/// Render an [`crate::EconomyError`] under the existence-hiding rule (§4.5,
/// D-E3-6), against **every** resource the refusal touches:
///
/// 1. the command's own `subject` (as before),
/// 2. every resource the ERROR names ([`crate::EconomyError::named_resources`]) —
///    the cross-resource half: `@econ/pay <mine>=<x> 1` answers about the
///    DESTINATION, and rendering that raw distinguished "no such account: 4242" from
///    "account 3 is not active" for accounts the actor may not see either way,
/// 3. the other resources the COMMAND named (`also`), for refusals that name
///    nothing but are policy answers about them
///    ([`crate::EconomyError::concerns_named_subjects`]):
///    `@asset/maintenance <mine>=<hidden wallet>` refused as "scope and owner do
///    not agree", which is an answer about a wallet the actor cannot see.
///
/// A cap or shape complaint keeps its own text (see `concerns_named_subjects`) —
/// hiding a payer's own operational error behind "No such account." would be a
/// disclosure rule eating the game.
fn refuse(ctx: &Ctx<'_>, subject: NamedResource, also: &[NamedResource], e: &crate::plan::EconomyError) -> CmdError {
    let named = e.named_resources();
    let extra: &[NamedResource] = if e.concerns_named_subjects() { also } else { &[] };
    for r in std::iter::once(subject).chain(named).chain(extra.iter().copied()) {
        if let Some(text) = hidden_text(ctx, r) {
            return CmdError::msg(text);
        }
    }
    CmdError::msg(e.to_string())
}

/// §4.5 view standing over a vault: staff, its owner, or `vault.view` on its scope.
fn may_view_vault(w: &World, actor: ObjId, id: VaultId) -> bool {
    if api::is_staff(w, actor) {
        return true;
    }
    let Some(store) = w.module_state::<EconomyStore>() else { return false };
    let Some(v) = store.snapshot().vault(id) else { return false };
    if v.owner == PrincipalId::Object(actor) {
        return true;
    }
    let Some(scope) = v.scope else { return false };
    api::scope_coherent(w, Some(scope), &v.owner) && api::holds_capability(w, actor, "vault.view", scope)
}

/// **The existence-hiding rule lives here, once** (§4.5, D-E3-6): any refusal that
/// touches an asset the actor may not VIEW renders as the nonexistent-asset text, so
/// "denied" and "there is no such thing" are indistinguishable. Only a view-
/// authorized actor sees the specific error.
fn asset_refusal(ctx: &Ctx<'_>, object: ObjId, detail: impl std::fmt::Display) -> CmdError {
    if may_view_asset(ctx.w, ctx.actor, object) {
        CmdError::msg(detail.to_string())
    } else {
        CmdError::msg("No such asset.")
    }
}
/// The vault half of the same rule.
fn vault_refusal(ctx: &Ctx<'_>, id: VaultId, detail: impl std::fmt::Display) -> CmdError {
    if may_view_vault(ctx.w, ctx.actor, id) {
        CmdError::msg(detail.to_string())
    } else {
        CmdError::msg("No such vault.")
    }
}

/// A plan carrying only title/custody ops (the effect-only shape invariant 10
/// admits) attributed to the acting player.
fn ops_plan(ctx: &Ctx<'_>, asset_ops: Vec<AssetOp>, custody_ops: Vec<CustodyOp>) -> EconomyPlan {
    let mut p = EconomyPlan::new(
        cmd_key(ctx),
        PrincipalId::Object(ctx.actor),
        EconomicCause::Command { serial: 0 },
        Vec::new(),
    );
    p.asset_ops = asset_ops;
    p.custody_ops = custody_ops;
    p
}

/// Resolve `<obj>` for an asset verb. A name that matches nothing renders as the
/// nonexistent-asset text (never "I don't see that here", which would leak).
fn asset_target(ctx: &Ctx<'_>, s: &str) -> Result<ObjId, CmdError> {
    match_object(ctx.w, ctx.actor, s).map_err(|_| CmdError::msg("No such asset."))
}

/// The post tail for an asset/custody verb, with §4.5 rendering.
fn finish_asset(
    ctx: &mut Ctx<'_>,
    object: ObjId,
    outcome: Result<PostOutcome, crate::plan::EconomyError>,
    verb: &str,
) -> Result<(), CmdError> {
    finish_ops(ctx, NamedResource::Asset(object), &[], outcome, verb)
}
fn finish_vault(
    ctx: &mut Ctx<'_>,
    vault: VaultId,
    outcome: Result<PostOutcome, crate::plan::EconomyError>,
    verb: &str,
) -> Result<(), CmdError> {
    finish_ops(ctx, NamedResource::Vault(vault), &[], outcome, verb)
}

/// The shared post tail: one confirmation shape, and every refusal through the
/// existence-hiding renderer against the command's subject, the resources the error
/// names, and the other resources the command named (`also`).
fn finish_ops(
    ctx: &mut Ctx<'_>,
    subject: NamedResource,
    also: &[NamedResource],
    outcome: Result<PostOutcome, crate::plan::EconomyError>,
    verb: &str,
) -> Result<(), CmdError> {
    match outcome {
        Ok(PostOutcome::Posted(id)) => {
            info(ctx, format!("{verb} (transaction #{id})."));
            Ok(())
        }
        Ok(PostOutcome::AlreadyPosted(id)) => {
            info(ctx, format!("Already posted as transaction #{id}."));
            Ok(())
        }
        Ok(PostOutcome::NeedsApproval(id)) => {
            info(ctx, format!("Approval required: request #{id}."));
            Ok(())
        }
        Err(e) => Err(refuse(ctx, subject, also, &e)),
    }
}

/// Emit a capped list with the §4.5 "N more" tail.
fn emit_list(ctx: &mut Ctx<'_>, mut rows: Vec<String>, empty: &str) {
    if rows.is_empty() {
        info(ctx, empty.to_string());
        return;
    }
    let extra = rows.len().saturating_sub(LIST_CAP);
    rows.truncate(LIST_CAP);
    for r in rows {
        info(ctx, r);
    }
    if extra > 0 {
        info(ctx, format!("({extra} more)"));
    }
}

// ---- @instrument ----------------------------------------------------------

fn cmd_instrument(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_admin(ctx)?;
    if ctx.switches.has("register") {
        instrument_register(ctx)
    } else if ctx.switches.has("list") {
        instrument_list(ctx)
    } else {
        Err(CmdError::msg("Usage: @instrument/register|list"))
    }
}

fn instrument_register(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let code = ctx.args.lhs().trim();
    let id = InstrumentId::new(code).ok_or_else(|| CmdError::msg("Bad instrument code (use [a-z0-9_.-])."))?;
    let scale: u8 = ctx
        .args
        .rhs()
        .and_then(|s| s.trim().parse().ok())
        .ok_or_else(|| CmdError::msg("Usage: @instrument/register <code>=<scale>"))?;
    if scale > crate::MAX_SCALE {
        return Err(CmdError::msg(format!("Scale too large (max {}).", crate::MAX_SCALE)));
    }
    if ctx.w.module_state::<EconomyStore>().is_some_and(|s| s.snapshot().instrument(&id).is_some()) {
        return Err(CmdError::msg("That instrument is already registered."));
    }
    let def = InstrumentDef {
        id: id.clone(),
        ruleset: SmolStr::new_static("staff"),
        code: SmolStr::new(code),
        kind: InstrumentKind::Currency,
        scale,
        transferable: true,
        negative_policy: NegativePolicy::SystemOnly,
        status: InstrumentStatus::Active,
        revision: 1,
    };
    api::register_instrument(ctx, def);
    info(ctx, format!("Registered instrument {id} (scale {scale})."));
    Ok(())
}

fn instrument_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lines: Vec<String> = {
        let store = ctx.w.module_state::<EconomyStore>().expect("economy store");
        let mut v: Vec<String> = store
            .snapshot()
            .instruments()
            .map(|i| format!("{} — scale {}, {:?}, {:?}", i.id, i.scale, i.kind, i.status))
            .collect();
        if v.is_empty() {
            v.push("No instruments.".into());
        }
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

// ---- @account -------------------------------------------------------------

fn cmd_account(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // Gating is per-subcommand from E3b2 on: the WRITES stay staff (opening a wallet
    // mints an owner binding; an approval rule is policy), `close` asks the economy
    // who may spend from it, and the READS are the §4.5 bounded non-staff surface.
    if ctx.switches.has("open") {
        require_admin(ctx)?;
        refuse_if_frozen(ctx)?;
        account_open(ctx, false)
    } else if ctx.switches.has("system") {
        require_admin(ctx)?;
        refuse_if_frozen(ctx)?;
        account_open(ctx, true)
    } else if ctx.switches.has("approval") {
        require_admin(ctx)?;
        refuse_if_frozen(ctx)?;
        account_approval(ctx)
    } else if ctx.switches.has("unfreeze") {
        require_admin(ctx)?;
        account_unfreeze(ctx)
    } else if ctx.switches.has("close") {
        account_close(ctx)
    } else if ctx.switches.has("show") {
        account_show(ctx)
    } else if ctx.switches.has("list") {
        account_list(ctx)
    } else {
        Err(CmdError::msg("Usage: @account/open|system|approval|close|unfreeze|show|list"))
    }
}

fn account_open(ctx: &mut Ctx<'_>, system: bool) -> Result<(), CmdError> {
    let instrument_code = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Usage: @account/open <obj>=<instrument>"))?;
    let instrument = InstrumentId::new(instrument_code).ok_or_else(|| CmdError::msg("Bad instrument code."))?;
    if ctx.w.module_state::<EconomyStore>().is_none_or(|s| s.snapshot().instrument(&instrument).is_none()) {
        return Err(CmdError::msg("No such instrument (register it first)."));
    }
    let lhs = ctx.args.lhs().trim().to_string();
    let (owner, name, credit, scope) = if system {
        let sys = SystemPrincipalId::new(&lhs).ok_or_else(|| CmdError::msg("Bad system principal name (use [a-z0-9_.-])."))?;
        (PrincipalId::System(sys), SmolStr::new(&lhs), CreditPolicy::unbounded(), None)
    } else if let Some(spec) = lhs.strip_prefix("org:") {
        // `org:<org>[/<scope>]` — the divisional wallet (§4.6). The compartment is
        // the policy anchor: everything about who may spend from this account is a
        // question about `scope`, answered live by org.
        let (org_s, scope_s) = match spec.split_once('/') {
            Some((o, s)) => (o, Some(s)),
            None => (spec, None),
        };
        let org = resolve_org(ctx, org_s).ok_or_else(|| CmdError::msg("No such org."))?;
        let (scope, label) = {
            let store = ctx.w.module_state::<mush_org::OrgStore>().ok_or_else(|| CmdError::msg("No organizations."))?;
            let state = store.state();
            let o = state.org(org).expect("resolved org exists");
            let sid = match scope_s {
                None => o.root_scope,
                Some(s) => parse_scope(s).ok_or_else(|| CmdError::msg("Bad scope id."))?,
            };
            let sc = state.scope(sid).ok_or_else(|| CmdError::msg("No such scope."))?;
            (sid, format!("{}/{}", o.name, sc.name))
        };
        let owner = PrincipalId::Organization(org);
        // Invariant 18 at open: the scope must belong to THIS org and be live (and
        // so must the org). The engine proves only the structural half.
        if !api::scope_coherent(ctx.w, Some(scope), &owner) {
            return Err(CmdError::msg(
                "That scope does not belong to that organization, or the scope/org is not active.",
            ));
        }
        (owner, SmolStr::new(label), CreditPolicy::forbid(), Some(scope))
    } else {
        let obj = match_object(ctx.w, ctx.actor, &lhs)?;
        (PrincipalId::Object(obj), SmolStr::new(display_name(ctx.w, obj)), CreditPolicy::forbid(), None)
    };
    let id = ctx.w.module_state_mut::<EconomyStore>().expect("economy store").alloc_account();
    let account = Account {
        id,
        owner: owner.clone(),
        instrument: instrument.clone(),
        scope,
        name,
        status: AccountStatus::Active,
        posted_balance: Amount::ZERO,
        held_amount: Amount::ZERO,
        credit,
        last_journal_seq: 0,
        revision: 1,
        // High-value escalation is opt-in per account (E3b2 sets it).
        approval: None,
    };
    api::open_account(ctx, account);
    let where_ = scope.map_or_else(String::new, |s| format!(" on scope #{s}"));
    info(ctx, format!(
        "Opened account #{id} ({instrument}){where_}{}.",
        if system { " [system source/sink]" } else { "" }
    ));
    // A control doc + the allocator meta: no money moved, so the ORDINARY durable
    // gate (never the strict money one). Without it the confirmation — and the
    // account id staff are about to hand out — can outrun its own durable write.
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `@account/show <id>` — §4.5's bounded non-staff read. A non-viewer gets the
/// SAME text as for an id that does not exist; a viewer gets id, instrument, name,
/// status, posted balance, and the approval rule if any. Staff additionally see the
/// owner, the compartment, and the projection bookkeeping.
fn account_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(id) = parse_account(ctx.args.lhs()) else {
        return Err(CmdError::msg("No such account."));
    };
    if !may_view_account(ctx.w, ctx.actor, id) {
        return Err(CmdError::msg("No such account."));
    }
    let staff = is_admin(ctx.w, ctx.actor);
    let (a, scale) = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No such account."))?;
        let a = store.snapshot().account(id).ok_or_else(|| CmdError::msg("No such account."))?.clone();
        let scale = store.snapshot().instrument(&a.instrument).map_or(0, |i| i.scale);
        (a, scale)
    };
    let mut lines = vec![
        format!("account #{} \"{}\" ({}) [{:?}]", a.id, a.name, a.instrument, a.status),
        format!("  balance {}", a.posted_balance.to_decimal_string(scale)),
    ];
    if let Some(rule) = &a.approval {
        lines.push(format!(
            "  approval above {}: {}",
            rule.threshold.to_decimal_string(scale),
            render_policy(ctx, &rule.policy)
        ));
    }
    if staff {
        lines.push(format!("  owner {}", render_principal(ctx, &a.owner)));
        if let Some(s) = a.scope {
            lines.push(format!("  scope #{s}"));
        }
        lines.push(format!("  held {}", a.held_amount.to_decimal_string(scale)));
        lines.push(format!("  last journal seq {}, revision {}", a.last_journal_seq, a.revision));
    }
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn render_policy(ctx: &Ctx<'_>, p: &ApprovalPolicy) -> String {
    match p {
        ApprovalPolicy::OneAuthorizedActor => "one authorized actor (inert)".to_string(),
        ApprovalPolicy::Threshold { approvals, from_role } => match from_role {
            None => format!("{approvals} approval(s)"),
            Some(r) => {
                let name = ctx
                    .w
                    .module_state::<mush_org::OrgStore>()
                    .and_then(|s| s.state().role(*r).map(|x| x.name.to_string()));
                format!("{approvals} approval(s) from role {}", name.unwrap_or_else(|| format!("#{r}")))
            }
        },
        ApprovalPolicy::NamedSet { approvals, actors } => format!(
            "{approvals} approval(s) from {}",
            actors.iter().map(|o| format!("#{}", o.0)).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// `@account/list` — every account the actor may VIEW, capped at 20 (§4.5). An
/// actor with no standing sees an empty list, never a count of what is hidden.
fn account_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let staff = is_admin(ctx.w, ctx.actor);
    let ids: Vec<AccountId> = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No economy."))?;
        store.snapshot().accounts().map(|a| a.id).collect()
    };
    let rows: Vec<String> = ids
        .into_iter()
        .filter(|id| may_view_account(ctx.w, ctx.actor, *id))
        .map(|id| {
            let store = ctx.w.module_state::<EconomyStore>().expect("economy store");
            let a = store.snapshot().account(id).expect("listed above");
            let scale = store.snapshot().instrument(&a.instrument).map_or(0, |i| i.scale);
            format!(
                "#{} {} {} = {} [{:?}]{}",
                a.id,
                a.name,
                a.instrument,
                a.posted_balance.to_decimal_string(scale),
                a.status,
                if a.approval.is_some() { " (approval rule)" } else { "" }
            )
        })
        .collect();
    emit_list(ctx, rows, "No accounts.");
    if staff && ctx.w.module_state::<EconomyStore>().is_some_and(EconomyStore::is_frozen) {
        info(ctx, "(store [ECONOMY FROZEN])".to_string());
    }
    Ok(())
}

// ---- @account/close | unfreeze | approval (E3b2) ---------------------------

/// `@account/close <id>` — zero balance, `Active`, and the authority to spend from
/// it: an org wallet needs staff or `economy.account.post` on its compartment (you
/// may not retire a division's wallet just because you can see it), a personal one
/// needs staff or its owner.
fn account_close(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(id) = parse_account(ctx.args.lhs()) else {
        return Err(CmdError::msg("No such account."));
    };
    if !may_view_account(ctx.w, ctx.actor, id) {
        return Err(CmdError::msg("No such account."));
    }
    let a = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No such account."))?;
        store.snapshot().account(id).ok_or_else(|| CmdError::msg("No such account."))?.clone()
    };
    let staff = is_admin(ctx.w, ctx.actor);
    let permitted = staff
        || match a.scope {
            Some(scope) => {
                api::scope_coherent(ctx.w, Some(scope), &a.owner)
                    && api::holds_capability(ctx.w, ctx.actor, "economy.account.post", scope)
            }
            None => a.owner == PrincipalId::Object(ctx.actor),
        };
    if !permitted {
        return Err(account_refusal(ctx, id, "You may not close that account."));
    }
    if a.status != AccountStatus::Active {
        return Err(account_refusal(ctx, id, "That account is not active."));
    }
    if !a.posted_balance.is_zero() || !a.held_amount.is_zero() {
        return Err(account_refusal(ctx, id, "That account still holds a balance."));
    }
    api::set_account_status(ctx, id, AccountStatus::Closed);
    info(ctx, format!("Account #{id} closed."));
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `@account/unfreeze <id>` — staff recovery (E2 shipped no unfreeze). A doc write,
/// never journaled, and deliberately no `revision` bump (see `set_account_status`).
fn account_unfreeze(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_account(ctx.args.lhs()).ok_or_else(|| CmdError::msg("Usage: @account/unfreeze <id>"))?;
    let status = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No such account."))?;
        store.snapshot().account(id).ok_or_else(|| CmdError::msg("No such account."))?.status
    };
    if status != AccountStatus::Frozen {
        return Err(CmdError::msg("That account is not frozen."));
    }
    if ctx.w.module_state::<EconomyStore>().is_some_and(EconomyStore::is_frozen) {
        return Err(CmdError::msg(
            "The whole economy is frozen; a per-account unfreeze would not restore posting.",
        ));
    }
    api::set_account_status(ctx, id, AccountStatus::Active);
    info(ctx, format!("Account #{id} unfrozen."));
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `@account/approval <id>=<threshold> <policy>` (staff) — the per-account
/// high-value escalation of D-E3-2.
fn account_approval(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @account/approval <id>=<threshold> threshold:<n>[:<role>] | \
                         named:<n>:<obj>,<obj>… | @account/approval <id>=none";
    let id = parse_account(ctx.args.lhs()).ok_or_else(|| CmdError::msg(USAGE))?;
    let account = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No such account."))?;
        store.snapshot().account(id).ok_or_else(|| CmdError::msg("No such account."))?.clone()
    };
    let rhs = ctx.args.rhs().map(str::trim).ok_or_else(|| CmdError::msg(USAGE))?;
    if rhs.eq_ignore_ascii_case("none") {
        api::set_account_approval(ctx, id, None);
        info(ctx, format!("Cleared the approval rule on account #{id}."));
        ctx.gate_on_durable_ack();
        return Ok(());
    }
    let mut words = rhs.split_whitespace();
    let threshold = words.next().and_then(parse_amount).ok_or_else(|| CmdError::msg(USAGE))?;
    if threshold.is_negative() {
        return Err(CmdError::msg("A threshold must not be negative."));
    }
    let spec = words.next().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
    if spec.eq_ignore_ascii_case("none") {
        api::set_account_approval(ctx, id, None);
        info(ctx, format!("Cleared the approval rule on account #{id}."));
        ctx.gate_on_durable_ack();
        return Ok(());
    }
    let policy = parse_policy(ctx, &account, &spec)?;
    api::set_account_approval(ctx, id, Some(ApprovalRule { threshold, policy: policy.clone() }));
    let scale = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().instrument(&account.instrument).map(|i| i.scale))
        .unwrap_or(0);
    let rendered = render_policy(ctx, &policy);
    info(ctx, format!(
        "Account #{id}: a net debit above {} now needs {rendered}.",
        threshold.to_decimal_string(scale)
    ));
    info(ctx, "That is a per-transaction escalation, not a budget.".to_string());
    // The rule is read live at every post, so the confirmation must not outrun the
    // doc that carries it (non-strict: policy, not money).
    ctx.gate_on_durable_ack();
    Ok(())
}

/// Parse `threshold:<n>[:<role>]` / `named:<n>:<obj>,<obj>…`, refusing the two
/// shapes that can never gate anything.
fn parse_policy(ctx: &Ctx<'_>, account: &Account, spec: &str) -> Result<ApprovalPolicy, CmdError> {
    let lower = spec.to_ascii_lowercase();
    if lower == "one" || lower == "oneauthorizedactor" {
        // A11: the one authorized actor is the poster, and four-eyes bars them, so
        // this rule blocks every payment forever rather than escalating any.
        return Err(CmdError::msg(
            "`one` can never be satisfied: the one authorized actor is the payer, and a requester \
             never approves their own request. Use threshold:1 (one OTHER person) or none.",
        ));
    }
    let (kind, rest) = lower.split_once(':').ok_or_else(|| CmdError::msg("Bad policy (use threshold:<n>[:<role>] or named:<n>:<obj>,…)."))?;
    match kind {
        "threshold" => {
            let (n_s, role_s) = match rest.split_once(':') {
                Some((n, r)) => (n, Some(r)),
                None => (rest, None),
            };
            let approvals = parse_approvals(n_s)?;
            let from_role = match role_s {
                None => None,
                Some(r) => Some(resolve_role(ctx, account, r)?),
            };
            Ok(ApprovalPolicy::Threshold { approvals, from_role })
        }
        "named" => {
            let (n_s, list) = rest
                .split_once(':')
                .ok_or_else(|| CmdError::msg("Usage: named:<n>:<obj>,<obj>…"))?;
            let approvals = parse_approvals(n_s)?;
            let mut actors: std::collections::BTreeSet<ObjId> = std::collections::BTreeSet::new();
            for token in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                let obj = match_object(ctx.w, ctx.actor, token)
                    .map_err(|_| CmdError::msg(format!("No such object: {token}")))?;
                actors.insert(obj);
            }
            if actors.is_empty() {
                return Err(CmdError::msg("A named set needs at least one actor."));
            }
            if usize::from(approvals) > actors.len() {
                return Err(CmdError::msg(
                    "That named set is smaller than the approvals it demands; it could never be satisfied.",
                ));
            }
            Ok(ApprovalPolicy::NamedSet { approvals, actors })
        }
        _ => Err(CmdError::msg("Bad policy (use threshold:<n>[:<role>] or named:<n>:<obj>,…).")),
    }
}

fn parse_approvals(s: &str) -> Result<u8, CmdError> {
    let n: u8 = s.trim().parse().map_err(|_| CmdError::msg("The approval count must be 1-255."))?;
    if n == 0 {
        // A zero-approval rule is satisfied by nothing at all — it escalates and
        // then waves the plan through, which reads as protection and is not.
        return Err(CmdError::msg("An approval count of 0 gates nothing; use `none` to clear the rule."));
    }
    Ok(n)
}

/// D-E3-11: a `from_role` policy is only meaningful against a REAL `OrgMembership`,
/// so the role must exist in the account's own organization.
fn resolve_role(ctx: &Ctx<'_>, account: &Account, s: &str) -> Result<mush_org::RoleId, CmdError> {
    let Some(scope) = account.scope else {
        return Err(CmdError::msg(
            "That account has no organization compartment, so a role-based policy has no roles to name.",
        ));
    };
    let store = ctx.w.module_state::<mush_org::OrgStore>().ok_or_else(|| CmdError::msg("No organizations."))?;
    let state = store.state();
    let org = state.scope(scope).ok_or_else(|| CmdError::msg("That account's scope no longer exists."))?.org;
    let s = s.trim();
    if let Ok(n) = s.trim_start_matches('#').parse::<u64>() {
        let id = mush_org::RoleId(n);
        if state.role(id).is_some_and(|r| r.org == org) {
            return Ok(id);
        }
        return Err(CmdError::msg("No such role in that organization."));
    }
    state
        .roles_in(org)
        .find(|r| r.name.eq_ignore_ascii_case(s))
        .map(|r| r.id)
        .ok_or_else(|| CmdError::msg("No such role in that organization."))
}

// ---- @econ ----------------------------------------------------------------

fn cmd_econ(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("pay") {
        econ_pay(ctx)
    } else if ctx.switches.has("mint") {
        require_admin(ctx)?;
        econ_mint(ctx)
    } else if ctx.switches.has("reverse") {
        require_admin(ctx)?;
        econ_reverse(ctx)
    } else if ctx.switches.has("audit") {
        require_admin(ctx)?;
        econ_audit(ctx)
    } else if ctx.switches.has("approve") {
        econ_decide(ctx, Verdict::Approved)
    } else if ctx.switches.has("deny") {
        econ_decide(ctx, Verdict::Denied)
    } else if ctx.switches.has("approvals") {
        econ_approvals(ctx)
    } else if ctx.switches.has("unfreeze") {
        require_admin(ctx)?;
        econ_unfreeze(ctx)
    } else {
        Err(CmdError::msg("Usage: @econ/pay|mint|reverse|audit|approve|deny|approvals|unfreeze"))
    }
}

/// `@econ/unfreeze` — staff recovery for the **whole-economy** latch (E3 verify
/// finding H). The latch is durable (`meta.frozen`), so this is the only thing that
/// clears it: a boot that froze on drift also heals the drifted projections and
/// persists them, which means no later boot would ever re-derive the freeze.
///
/// It deliberately does NOT unfreeze the accounts, assets, or vaults that the same
/// recovery boot latched: those name which records need attention, and clearing them
/// wholesale would erase the reason anyone froze.
fn econ_unfreeze(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.w.module_state::<EconomyStore>().is_none_or(|s| !s.is_frozen()) {
        return Err(CmdError::msg("The economy is not frozen."));
    }
    api::set_economy_frozen(ctx, false);
    info(ctx, "The whole-economy freeze is cleared. Posting is live again.".to_string());
    info(
        ctx,
        "Accounts, assets and vaults frozen by the recovery boot stay frozen: clear them with \
         @account/unfreeze, @asset/unfreeze and @vault/unfreeze."
            .to_string(),
    );
    // A control-doc write, not money: the ordinary durable gate (§4.2).
    ctx.gate_on_durable_ack();
    Ok(())
}

// ---- approvals (E3b2 §4.6) ------------------------------------------------

/// The unknown-id text every approval refusal collapses to for an actor without
/// standing (§4.5): an approval request names accounts and amounts, so even
/// confirming that id #7 exists is a disclosure.
const NO_SUCH_REQUEST: &str = "No such approval request.";

fn parse_approval_id(s: &str) -> Option<ApprovalRequestId> {
    s.trim().trim_start_matches('#').parse::<u64>().ok().map(ApprovalRequestId)
}

/// Whether `actor` may act on a request: would their approval COUNT for at least
/// one of its accounts, under that account's live rule? That is exactly
/// `authorize_approver`, so standing and counting can never drift apart.
///
/// The requester is not excluded here — they may look at and even record a decision
/// on their own request; the engine simply never counts it (four-eyes).
fn may_act_on_request(w: &World, actor: ObjId, request: &crate::record::ApprovalRequest) -> bool {
    let Some(store) = w.module_state::<EconomyStore>() else { return false };
    let snap = store.snapshot();
    let auth = api::WorldAuth { w };
    request.accounts.iter().any(|id| {
        snap.account(*id).is_some_and(|a| {
            a.approval
                .as_ref()
                .is_some_and(|rule| auth.authorize_approver(actor, a, &rule.policy))
        })
    })
}

/// `@econ/approve <id>` / `@econ/deny <id>`. Records a [`Decision`] either way (the
/// audit trail keeps denials); a denial `Cancel`s the request, and approving twice
/// is a no-op rather than a second vote.
fn econ_decide(ctx: &mut Ctx<'_>, verdict: Verdict) -> Result<(), CmdError> {
    let unknown = || CmdError::msg(NO_SUCH_REQUEST);
    let id = parse_approval_id(ctx.args.lhs()).ok_or_else(unknown)?;
    let mut request = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(unknown)?;
        store.snapshot().approval(id).ok_or_else(unknown)?.clone()
    };
    // Standing FIRST: only once it is established may a status-specific message be
    // shown, or the messages themselves would enumerate the request table.
    if !may_act_on_request(ctx.w, ctx.actor, &request) {
        return Err(unknown());
    }
    let now = ctx.w.now();
    if request.status != ApprovalStatus::Pending {
        return Err(CmdError::msg(format!("Request #{id} is already {}.", status_word(request.status))));
    }
    if now >= request.expires_at {
        return Err(CmdError::msg(format!("Request #{id} has expired; the payer must submit a fresh one.")));
    }
    let me = ctx.actor;
    if verdict == Verdict::Approved && request.approvals.iter().any(|a| a.approver == me) {
        info(ctx, format!("You have already approved request #{id}."));
        return Ok(());
    }
    if request.decisions.len() >= crate::MAX_APPROVERS_PER_REQUEST {
        return Err(CmdError::msg(format!(
            "Request #{id} has already recorded the maximum of {} decisions.",
            crate::MAX_APPROVERS_PER_REQUEST
        )));
    }
    request.decisions.push(Decision { actor: me, at: now, verdict });
    match verdict {
        Verdict::Approved => {
            if request.approvals.len() >= crate::MAX_APPROVERS_PER_REQUEST {
                return Err(CmdError::msg(format!(
                    "Request #{id} has already recorded the maximum of {} approvals.",
                    crate::MAX_APPROVERS_PER_REQUEST
                )));
            }
            request.approvals.push(Approval { approver: me, at: now });
        }
        Verdict::Denied => request.status = ApprovalStatus::Cancelled,
    }
    request.revision += 1;
    let requester_is_me = request.requested_by == PrincipalId::Object(me);
    api::put_approval(ctx, request);
    match verdict {
        Verdict::Approved => {
            info(ctx, format!("Approved request #{id}."));
            if requester_is_me {
                info(ctx, "(It is your own request, so it does not count toward the policy.)".to_string());
            }
        }
        Verdict::Denied => info(ctx, format!("Denied request #{id}; it is cancelled.")),
    }
    Ok(())
}

fn status_word(s: ApprovalStatus) -> &'static str {
    match s {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Consumed { .. } => "spent",
        ApprovalStatus::Cancelled => "cancelled",
        ApprovalStatus::Expired => "expired",
    }
}

/// `@econ/approvals` — the live requests this actor may act on, capped at 20.
fn econ_approvals(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let now = ctx.w.now();
    let live: Vec<crate::record::ApprovalRequest> = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No economy."))?;
        store
            .snapshot()
            .approvals()
            .filter(|r| r.status == ApprovalStatus::Pending && now < r.expires_at)
            .cloned()
            .collect()
    };
    let rows: Vec<String> = live
        .into_iter()
        .filter(|r| may_act_on_request(ctx.w, ctx.actor, r))
        .map(|r| {
            let mine = if r.requested_by == PrincipalId::Object(ctx.actor) { " (yours)" } else { "" };
            let already = if r.approvals.iter().any(|a| a.approver == ctx.actor) { " [approved]" } else { "" };
            format!(
                "#{} — requested by {}, {} approval(s) so far, account(s) {}{mine}{already}",
                r.id,
                render_principal(ctx, &r.requested_by),
                r.approvals.len(),
                r.accounts.iter().map(|a| format!("#{a}")).collect::<Vec<_>>().join(", "),
            )
        })
        .collect();
    emit_list(ctx, rows, "No approval requests await you.");
    Ok(())
}

/// Parse `<amount>` in an instrument's minor units (integer minor units for E2b —
/// staff type the raw minor value; the display formats the scale).
fn parse_amount(s: &str) -> Option<Amount> {
    s.trim().parse::<i64>().ok().map(Amount::from_minor)
}

fn two_accounts_and_amount(ctx: &Ctx<'_>, usage: &str) -> Result<(AccountId, AccountId, Amount), CmdError> {
    let from = parse_account(ctx.args.lhs()).ok_or_else(|| CmdError::msg(usage.to_string()))?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(usage.to_string()))?;
    let mut it = rhs.split_whitespace();
    let to = it.next().and_then(parse_account).ok_or_else(|| CmdError::msg(usage.to_string()))?;
    let amount = it.next().and_then(parse_amount).ok_or_else(|| CmdError::msg(usage.to_string()))?;
    if amount.is_zero() || amount.is_negative() {
        return Err(CmdError::msg("Amount must be positive."));
    }
    Ok((from, to, amount))
}

/// `@econ/pay <from>=<to> <amount> [asset=<obj>] [from_vault=<id>|to_vault=<id>]`
///
/// One plan, one durable bundle: money, and optionally the title and custody of one
/// asset. **Arity 1** (D-E3-4): an E3 plan is single-authority, so the payer and the
/// asset's title holder must be the same principal unless the actor is staff — an
/// actual sale between two principals needs counterparty consent, which is E4.
fn econ_pay(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @econ/pay <from>=<to> <amount> [asset=<obj>] [from_vault=<id>|to_vault=<id>]";
    let from = parse_account(ctx.args.lhs()).ok_or_else(|| CmdError::msg(USAGE))?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let (rest, opts) = parse_opts(rhs, &["asset", "from_vault", "to_vault"]);
    let mut it = rest.split_whitespace();
    let to = it.next().and_then(parse_account).ok_or_else(|| CmdError::msg(USAGE))?;
    let amount = it.next().and_then(parse_amount).ok_or_else(|| CmdError::msg(USAGE))?;
    if amount.is_zero() || amount.is_negative() {
        return Err(CmdError::msg("Amount must be positive."));
    }
    // A self-transfer moves nothing and nets to zero — the belt on the engine's
    // gross-debit authorization (a `[-n, +n]` pair on ONE account used to read as a
    // free credit and post against any account the actor could name). Refused BEFORE
    // any lookup, so it discloses nothing about either id.
    if from == to {
        return Err(CmdError::msg("An account cannot pay itself."));
    }
    // §4.4: a non-staff payer may name an account they OWN, or a SCOPED (org) one —
    // and only those two. The engine then decides the scoped case live, against the
    // org's capability + limit + compartment status.
    //
    // This is deliberately not "let the engine decide everything": a `System`
    // faucet is neither owned nor scoped, so it never reaches the engine from here,
    // and `WorldAuth::authorize` refuses it a second time even if some other caller
    // does reach it (A6 — two locks, because minting is the one irreversible act).
    let staff = is_admin(ctx.w, ctx.actor);
    if !staff {
        let source = ctx.w.module_state::<EconomyStore>().and_then(|s| s.snapshot().account(from).cloned());
        let eligible = source
            .as_ref()
            .is_some_and(|a| a.owner == PrincipalId::Object(ctx.actor) || a.scope.is_some());
        if !eligible {
            return Err(account_refusal(
                ctx,
                from,
                "You may only pay from your own account or an organization account you hold authority on.",
            ));
        }
    }

    // The optional title/custody legs — resolved BEFORE `post` (§4.2 step 6).
    let mut asset_ops: Vec<AssetOp> = Vec::new();
    let mut custody_ops: Vec<CustodyOp> = Vec::new();
    let mut asset_object: Option<ObjId> = None;
    if let Some(spec) = opts.get("asset") {
        let object = asset_target(ctx, spec)?;
        asset_object = Some(object);
        let (payer, recipient, to_scope, title_owner) = {
            let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No economy."))?;
            let snap = store.snapshot();
            let payer = snap.account(from).map(|a| a.owner.clone());
            let dest = snap.account(to).ok_or_else(|| CmdError::msg("No such account."))?;
            let title_owner = snap.asset(object).map(|a| a.title_owner.clone());
            (payer, dest.owner.clone(), dest.scope, title_owner)
        };
        let Some(title_owner) = title_owner else {
            return Err(asset_refusal(ctx, object, "No such asset."));
        };
        if !staff && payer.as_ref() != Some(&title_owner) {
            return Err(asset_refusal(
                ctx,
                object,
                "That is a two-party exchange: the payer and the title holder are different \
                 principals, and an E3 plan carries one authority. Counterparty consent is E4.",
            ));
        }
        asset_ops.push(AssetOp::TransferTitle { object, to: recipient, to_scope });
        match (opts.get("to_vault"), opts.get("from_vault")) {
            (Some(_), Some(_)) => {
                return Err(CmdError::msg("Use at most one of to_vault= and from_vault=."));
            }
            (Some(v), None) => {
                let vault = parse_vault_id(v).ok_or_else(|| CmdError::msg("No such vault."))?;
                custody_ops.push(CustodyOp::Deposit { object, vault });
            }
            (None, Some(v)) => {
                let vault = parse_vault_id(v).ok_or_else(|| CmdError::msg("No such vault."))?;
                custody_ops.push(CustodyOp::Withdraw { object, vault, to: ctx.actor });
            }
            (None, None) => {}
        }
    } else if opts.contains_key("to_vault") || opts.contains_key("from_vault") {
        return Err(CmdError::msg("A vault leg needs asset=<obj>."));
    }

    let neg = amount.checked_neg().ok_or_else(|| CmdError::msg("Amount out of range."))?;
    let mut plan = EconomyPlan::new(
        cmd_key(ctx),
        PrincipalId::Object(ctx.actor),
        EconomicCause::Command { serial: 0 },
        vec![
            RequestedLine { account: from, delta: neg, memo: SmolStr::default() },
            RequestedLine { account: to, delta: amount, memo: SmolStr::default() },
        ],
    );
    plan.asset_ops = asset_ops;
    plan.custody_ops = custody_ops;
    let outcome = api::post(ctx, &plan);
    match asset_object {
        // Both accounts ride along as `also`: a refusal about the money half of a
        // pay-with-asset plan must hide behind them exactly as a money-only pay does.
        Some(object) => finish_ops(
            ctx,
            NamedResource::Asset(object),
            &[NamedResource::Account(from), NamedResource::Account(to)],
            outcome,
            "Paid",
        ),
        None => finish_pay(ctx, from, to, outcome),
    }
}

/// The `@econ/pay` tail: the same outcomes as [`finish`], but every refusal is
/// rendered through the account existence-hiding rule (§4.5) — a payer with no
/// standing on the source account learns only that there is no such account, and a
/// refusal that answers about the DESTINATION hides behind it just the same.
fn finish_pay(
    ctx: &mut Ctx<'_>,
    from: AccountId,
    to: AccountId,
    outcome: Result<PostOutcome, crate::plan::EconomyError>,
) -> Result<(), CmdError> {
    match outcome {
        Ok(PostOutcome::Posted(id)) => {
            info(ctx, format!("Paid: transaction #{id} posted."));
            Ok(())
        }
        Ok(PostOutcome::AlreadyPosted(id)) => {
            info(ctx, format!("Already posted as transaction #{id}."));
            Ok(())
        }
        Ok(PostOutcome::NeedsApproval(id)) => {
            info(ctx, format!("Approval required: request #{id}. Nothing has moved."));
            info(ctx, "Once it is approved, run the same command again to post it.".to_string());
            Ok(())
        }
        Err(e) => Err(refuse(ctx, NamedResource::Account(from), &[NamedResource::Account(to)], &e)),
    }
}

fn econ_mint(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (source, to, amount) = two_accounts_and_amount(ctx, "Usage: @econ/mint <sysacct>=<to> <amount>")?;
    let neg = amount.checked_neg().ok_or_else(|| CmdError::msg("Amount out of range."))?;
    let plan = EconomyPlan::new(
        cmd_key(ctx),
        PrincipalId::Object(ctx.actor),
        EconomicCause::Staff { reason: "mint".into() },
        vec![
            RequestedLine { account: source, delta: neg, memo: SmolStr::new_static("mint") },
            RequestedLine { account: to, delta: amount, memo: SmolStr::new_static("grant") },
        ],
    );
    let outcome = api::post(ctx, &plan);
    finish(ctx, outcome, "Minted")
}

fn econ_reverse(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let txid = ctx
        .args
        .lhs()
        .trim()
        .trim_start_matches('#')
        .parse::<u64>()
        .map(TransactionId)
        .map_err(|_| CmdError::msg("Usage: @econ/reverse <txid>"))?;
    let outcome = api::reverse(ctx, txid, PrincipalId::Object(ctx.actor), EconomicCause::Staff { reason: "reversal".into() });
    finish(ctx, outcome, "Reversed")
}

fn econ_audit(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lines: Vec<String> = {
        let store = ctx.w.module_state::<EconomyStore>().expect("economy store");
        let all: Vec<&crate::record::EconomicTransaction> = store.transactions().collect();
        let mut v: Vec<String> = all
            .iter()
            .rev()
            .take(20)
            .map(|t| format!("  tx #{} seq {} ({} line(s)){}", t.id, t.journal_seq, t.lines.len(), t.reverses.map_or_else(String::new, |r| format!(" reverses #{r}"))))
            .collect();
        v.reverse();
        if v.is_empty() {
            v.push("No transactions.".into());
        }
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn finish(ctx: &mut Ctx<'_>, outcome: Result<PostOutcome, crate::plan::EconomyError>, verb: &str) -> Result<(), CmdError> {
    match outcome {
        Ok(PostOutcome::Posted(id)) => {
            info(ctx, format!("{verb}: transaction #{id} posted."));
            Ok(())
        }
        Ok(PostOutcome::AlreadyPosted(id)) => {
            info(ctx, format!("Already posted as transaction #{id}."));
            Ok(())
        }
        Ok(PostOutcome::NeedsApproval(id)) => {
            // E3b2 mints the request and serves `@account/approve`; E3a's seam
            // surfaces `ApprovalRequired` instead, so this arm is forward wiring.
            info(ctx, format!("Approval required: request #{id}."));
            Ok(())
        }
        Err(e) => Err(CmdError::msg(e.to_string())),
    }
}

// ---- @asset ---------------------------------------------------------------

fn cmd_asset(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("title") {
        asset_title(ctx)
    } else if ctx.switches.has("transfer") {
        asset_transfer(ctx)
    } else if ctx.switches.has("assign") {
        asset_assign(ctx)
    } else if ctx.switches.has("unassign") {
        asset_unassign(ctx)
    } else if ctx.switches.has("maintenance") {
        asset_maintenance(ctx)
    } else if ctx.switches.has("lien") {
        asset_lien(ctx)
    } else if ctx.switches.has("release") {
        asset_release(ctx)
    } else if ctx.switches.has("writeoff") {
        asset_writeoff(ctx)
    } else if ctx.switches.has("unfreeze") {
        asset_unfreeze(ctx)
    } else if ctx.switches.has("show") {
        asset_show(ctx)
    } else if ctx.switches.has("list") {
        asset_list(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: @asset/title|transfer|assign|unassign|maintenance|lien|release|writeoff|unfreeze|show|list",
        ))
    }
}

/// `<principal> [scope=<id>]` — the owner half shared by `title` and `transfer`.
fn owner_and_scope(ctx: &Ctx<'_>, rhs: &str) -> Result<(PrincipalId, Option<OrgScopeId>), CmdError> {
    let (rest, opts) = parse_opts(rhs, &["scope"]);
    let owner = parse_principal(ctx, rest.trim()).ok_or_else(|| CmdError::msg("No such principal."))?;
    let scope = match opts.get("scope") {
        Some(s) => Some(parse_scope(s).ok_or_else(|| CmdError::msg("Bad scope id."))?),
        None => None,
    };
    Ok((owner, scope))
}

fn asset_title(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let object = asset_target(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg("Usage: @asset/title <obj>=<principal> [scope=<id>]"))?;
    let (owner, scope) = owner_and_scope(ctx, rhs)?;
    let plan = ops_plan(ctx, vec![AssetOp::Title { object, owner, scope, catalog: None }], Vec::new());
    let outcome = api::post(ctx, &plan);
    finish_asset(ctx, object, outcome, "Title recorded")
}

fn asset_transfer(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let object = asset_target(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg("Usage: @asset/transfer <obj>=<principal> [scope=<id>]"))?;
    let (to, to_scope) = owner_and_scope(ctx, rhs)?;
    let plan = ops_plan(ctx, vec![AssetOp::TransferTitle { object, to, to_scope }], Vec::new());
    let outcome = api::post(ctx, &plan);
    finish_asset(ctx, object, outcome, "Title transferred")
}

fn asset_assign(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let object = asset_target(ctx, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg("Usage: @asset/assign <obj>=<principal> [purpose] [until=<unix>]"))?;
    let (rest, opts) = parse_opts(rhs, &["until"]);
    let mut words = rest.split_whitespace();
    let who = words.next().ok_or_else(|| CmdError::msg("Assign to whom?"))?;
    let to = parse_principal(ctx, who).ok_or_else(|| CmdError::msg("No such principal."))?;
    let purpose: String = words.collect::<Vec<_>>().join(" ");
    let valid_until = match opts.get("until") {
        Some(s) => Some(s.parse::<i64>().map_err(|_| CmdError::msg("Bad until= time."))?),
        None => None,
    };
    let plan = ops_plan(
        ctx,
        vec![AssetOp::Assign { object, to, purpose: SmolStr::new(purpose), valid_until }],
        Vec::new(),
    );
    let outcome = api::post(ctx, &plan);
    finish_asset(ctx, object, outcome, "Assigned")
}

fn asset_unassign(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let object = asset_target(ctx, ctx.args.lhs())?;
    let plan = ops_plan(ctx, vec![AssetOp::Unassign { object }], Vec::new());
    let outcome = api::post(ctx, &plan);
    finish_asset(ctx, object, outcome, "Assignment cleared")
}

fn asset_maintenance(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let object = asset_target(ctx, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .map(str::trim)
        .ok_or_else(|| CmdError::msg("Usage: @asset/maintenance <obj>=<account|none>"))?;
    let account = if rhs.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(parse_account(rhs).ok_or_else(|| CmdError::msg("No such account."))?)
    };
    let plan = ops_plan(ctx, vec![AssetOp::SetMaintenance { object, account }], Vec::new());
    let outcome = api::post(ctx, &plan);
    // The payer account is a subject too: invariant 14's refusal ("scope and owner
    // do not agree") is an answer ABOUT that account, and naming a wallet the actor
    // may not view is how a probe would find one.
    let also: Vec<NamedResource> = account.map(NamedResource::Account).into_iter().collect();
    finish_ops(ctx, NamedResource::Asset(object), &also, outcome, "Maintenance payer set")
}

fn asset_lien(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let object = asset_target(ctx, ctx.args.lhs())?;
    const USAGE: &str = "Usage: @asset/lien <obj>=<creditor> <amount> <instrument>";
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let mut words = rhs.split_whitespace();
    let creditor = words
        .next()
        .and_then(|s| parse_principal(ctx, s))
        .ok_or_else(|| CmdError::msg("No such principal."))?;
    let amount = words.next().and_then(parse_amount).ok_or_else(|| CmdError::msg(USAGE))?;
    let instrument = words
        .next()
        .and_then(InstrumentId::new)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let plan = ops_plan(
        ctx,
        vec![AssetOp::Encumber { object, encumbrance: Encumbrance::Lien { creditor, amount, instrument } }],
        Vec::new(),
    );
    let outcome = api::post(ctx, &plan);
    finish_asset(ctx, object, outcome, "Lien recorded")
}

fn asset_release(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let object = asset_target(ctx, ctx.args.lhs())?;
    let n: usize = ctx
        .args
        .rhs()
        .and_then(|s| s.trim().parse().ok())
        .filter(|n| *n >= 1)
        .ok_or_else(|| CmdError::msg("Usage: @asset/release <obj>=<n>   (n from @asset/show)"))?;
    // Resolve the exact recorded claim BEFORE `post` — `Release` is an exact match.
    let encumbrance = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No economy."))?;
        store
            .snapshot()
            .asset(object)
            .and_then(|a| a.encumbrances.get(n - 1).cloned())
    };
    let Some(encumbrance) = encumbrance else {
        return Err(asset_refusal(ctx, object, "No such encumbrance."));
    };
    let plan = ops_plan(ctx, vec![AssetOp::Release { object, encumbrance }], Vec::new());
    let outcome = api::post(ctx, &plan);
    finish_asset(ctx, object, outcome, "Claim released")
}

fn asset_writeoff(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let object = asset_target(ctx, ctx.args.lhs())?;
    let reason = ctx
        .args
        .rhs()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Usage: @asset/writeoff <obj>=<reason>"))?;
    let plan = ops_plan(ctx, vec![AssetOp::WriteOff { object, reason }], Vec::new());
    let outcome = api::post(ctx, &plan);
    finish_asset(ctx, object, outcome, "Written off")
}

/// Staff recovery. The `frozen` latch is doc authority (D-E3-1), so clearing it is a
/// plain doc write — never a journal effect.
fn asset_unfreeze(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_admin(ctx)?;
    let object = asset_target(ctx, ctx.args.lhs())?;
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_none_or(|s| s.snapshot().asset(object).is_none())
    {
        return Err(CmdError::msg("No such asset."));
    }
    api::set_asset_frozen(ctx, object, false, None);
    info(ctx, format!("Asset #{} unfrozen.", object.0));
    ctx.gate_on_durable_ack();
    Ok(())
}

fn asset_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let object = asset_target(ctx, ctx.args.lhs())?;
    if !may_view_asset(ctx.w, ctx.actor, object) {
        return Err(CmdError::msg("No such asset."));
    }
    let staff = is_admin(ctx.w, ctx.actor);
    let lines = {
        let a = {
            let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No such asset."))?;
            store.snapshot().asset(object).ok_or_else(|| CmdError::msg("No such asset."))?.clone()
        };
        let mut v = vec![format!(
            "asset #{} — title {} [{:?}]",
            a.object.0,
            render_principal(ctx, &a.title_owner),
            a.lifecycle
        )];
        match &a.assignment {
            Some(x) => v.push(format!("  assigned to {} ({})", render_principal(ctx, &x.assigned_to), x.purpose)),
            None => v.push("  unassigned".to_string()),
        }
        v.push(format!("  encumbrances: {}", a.encumbrances.len()));
        if staff {
            if let Some(s) = a.scope {
                v.push(format!("  scope #{s}"));
            }
            if let Some(m) = a.maintenance_account {
                v.push(format!("  maintenance account #{m}"));
            }
            for (i, e) in a.encumbrances.iter().enumerate() {
                v.push(format!("    {}. {}", i + 1, render_encumbrance(ctx, e)));
            }
            if let Some(acq) = a.acquisition {
                v.push(format!("  acquired by transaction #{} at {}", acq.transaction, acq.at));
            }
            v.push(format!("  last journal seq {}, revision {}", a.last_journal_seq, a.revision));
            if a.frozen {
                v.push(format!("  FROZEN ({})", a.frozen_reason.as_deref().unwrap_or("no reason")));
            }
        }
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn render_encumbrance(ctx: &Ctx<'_>, e: &Encumbrance) -> String {
    match e {
        Encumbrance::Lien { creditor, amount, instrument } => {
            format!("lien {} {} to {}", amount.minor(), instrument, render_principal(ctx, creditor))
        }
        Encumbrance::Disputed { claimant, reason } => {
            format!("disputed by {} ({reason})", render_principal(ctx, claimant))
        }
    }
}

fn asset_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // `owner=`/`scope=` carry their own `=`, which `ArgShape::EqPair` would eat as the
    // pair separator — read the untouched argument text instead.
    let raw = ctx.args.raw().to_string();
    let (_, opts) = parse_opts(&raw, &["owner", "scope"]);
    // §4.5: an unauthorized, hidden, or nonexistent filter yields an EMPTY list —
    // never an error that would confirm the filter's existence.
    let owner = match opts.get("owner") {
        Some(s) => match parse_principal(ctx, s) {
            Some(p) => Some(p),
            None => return empty_list(ctx, "No assets."),
        },
        None => None,
    };
    let scope = match opts.get("scope") {
        Some(s) => match parse_scope(s) {
            Some(sc) => Some(sc),
            None => return empty_list(ctx, "No assets."),
        },
        None => None,
    };
    let candidates: Vec<ObjId> = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No economy."))?;
        store
            .snapshot()
            .assets()
            .filter(|a| owner.as_ref().is_none_or(|o| a.title_owner == *o))
            .filter(|a| scope.is_none_or(|s| a.scope == Some(s)))
            .map(|a| a.object)
            .collect()
    };
    let rows: Vec<String> = candidates
        .into_iter()
        .filter(|o| may_view_asset(ctx.w, ctx.actor, *o))
        .map(|o| {
            let a = {
                let store = ctx.w.module_state::<EconomyStore>().expect("economy store");
                store.snapshot().asset(o).expect("listed above").clone()
            };
            format!(
                "#{} {} — {} [{:?}]{}",
                a.object.0,
                display_name(ctx.w, a.object),
                render_principal(ctx, &a.title_owner),
                a.lifecycle,
                if a.frozen { " FROZEN" } else { "" }
            )
        })
        .collect();
    emit_list(ctx, rows, "No assets.");
    Ok(())
}

fn empty_list(ctx: &mut Ctx<'_>, empty: &str) -> Result<(), CmdError> {
    info(ctx, empty.to_string());
    Ok(())
}

// ---- @vault ---------------------------------------------------------------

fn cmd_vault(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("create") {
        vault_create(ctx)
    } else if ctx.switches.has("deposit") {
        vault_deposit(ctx)
    } else if ctx.switches.has("withdraw") {
        vault_withdraw(ctx)
    } else if ctx.switches.has("show") {
        vault_show(ctx)
    } else if ctx.switches.has("list") {
        vault_list(ctx)
    } else if ctx.switches.has("freeze") {
        vault_set_status(ctx, VaultStatus::Frozen, "frozen")
    } else if ctx.switches.has("unfreeze") {
        vault_set_status(ctx, VaultStatus::Active, "unfrozen")
    } else if ctx.switches.has("close") {
        vault_close(ctx)
    } else {
        Err(CmdError::msg("Usage: @vault/create|deposit|withdraw|show|list|freeze|unfreeze|close"))
    }
}

fn vault_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_admin(ctx)?;
    refuse_if_frozen(ctx)?;
    const USAGE: &str =
        "Usage: @vault/create <name>=container:<obj>|interior:<obj> [owner=<principal>] [scope=<id>]";
    let name = ctx.args.lhs().trim().to_string();
    if name.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let (rest, opts) = parse_opts(rhs, &["owner", "scope"]);
    let spec = rest.trim();
    let backing = if let Some(o) = spec.strip_prefix("container:") {
        CustodyRef::Container(match_object(ctx.w, ctx.actor, o).map_err(|_| CmdError::msg("No such object."))?)
    } else if let Some(o) = spec.strip_prefix("interior:") {
        // E3c: legal as soon as SOME registered provider handles the backing
        // kind (`mush_sim::InteriorCustodyProvider`). Whether *this* room is a
        // usable interior is `backing_valid`'s question, below.
        CustodyRef::SpatialInterior(match_object(ctx.w, ctx.actor, o).map_err(|_| CmdError::msg("No such object."))?)
    } else {
        return Err(CmdError::msg(USAGE));
    };
    let backing_obj = backing.object().ok_or_else(|| CmdError::msg(USAGE))?;
    let owner = match opts.get("owner") {
        Some(s) => parse_principal(ctx, s).ok_or_else(|| CmdError::msg("No such principal."))?,
        None => PrincipalId::Object(ctx.actor),
    };
    let scope = match opts.get("scope") {
        Some(s) => Some(parse_scope(s).ok_or_else(|| CmdError::msg("Bad scope id."))?),
        None => None,
    };
    // Invariant 18 at create (§4.4), both halves.
    if !api::scope_coherent(ctx.w, scope, &owner) {
        return Err(CmdError::msg("That scope does not belong to that organization, or is not active."));
    }
    // One vault per backing, and never a backing another custody system owns.
    {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No economy."))?;
        if let Some(v) = store.snapshot().vault_by_backing(&backing) {
            return Err(CmdError::msg(format!("That object already backs vault #{}.", v.id)));
        }
        let providers = store.providers();
        if !providers.iter().any(|p| p.handles(&backing)) {
            return Err(CmdError::msg(if matches!(backing, CustodyRef::SpatialInterior(_)) {
                "Interior vaults are not available until the interior custody provider is wired."
            } else {
                "Nothing registered can hold a vault of that kind."
            }));
        }
        // Resolving WITH the object is the claim check: the provider that will
        // run this vault's ops is exempt (an interior vault's backing IS a space
        // the simulation owns — that is the point), but a backing some OTHER
        // system owns is refused, so a placed hull never becomes a footlocker.
        let resolved = crate::custody::resolve(&providers, ctx.w, &backing, Some(backing_obj)).map_err(|e| {
            match e {
                crate::custody::CustodyError::Claimed { provider, .. } => CmdError::msg(format!(
                    "That object is held by the {provider} system and cannot back a vault of that kind."
                )),
                other => CmdError::msg(other.to_string()),
            }
        })?;
        if resolved.backing_valid(ctx.w, &backing) != Some(true) {
            return Err(CmdError::msg("That object cannot back a vault."));
        }
    }
    let id = ctx.w.module_state_mut::<EconomyStore>().expect("economy store").alloc_vault();
    let vault = Vault {
        id,
        owner,
        scope,
        name: SmolStr::new(&name),
        backing,
        status: VaultStatus::Active,
        revision: 1,
    };
    api::create_vault(ctx, vault);
    info(ctx, format!("Created vault #{id} ({name}) backed by #{}.", backing_obj.0));
    ctx.gate_on_durable_ack();
    Ok(())
}

fn vault_deposit(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let object = asset_target(ctx, ctx.args.lhs())?;
    let vault = ctx
        .args
        .rhs()
        .and_then(parse_vault_id)
        .ok_or_else(|| CmdError::msg("Usage: @vault/deposit <obj>=<vault>"))?;
    let plan = ops_plan(ctx, Vec::new(), vec![CustodyOp::Deposit { object, vault }]);
    let outcome = api::post(ctx, &plan);
    finish_vault(ctx, vault, outcome, "Deposited")
}

fn vault_withdraw(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let object = asset_target(ctx, ctx.args.lhs())?;
    const USAGE: &str = "Usage: @vault/withdraw <obj>=<vault> [to=here]";
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let (rest, opts) = parse_opts(rhs, &["to"]);
    let vault = parse_vault_id(&rest).ok_or_else(|| CmdError::msg(USAGE))?;
    // Invariant 15 constrains `to` to the actor or the actor's location; the engine
    // re-checks live, this only spells the shorthand.
    let to = match opts.get("to").map(String::as_str) {
        None | Some("me") => ctx.actor,
        Some("here") => ctx
            .w
            .get(ctx.actor)
            .and_then(|o| o.location)
            .ok_or_else(|| CmdError::msg("You are nowhere."))?,
        Some(other) => match_object(ctx.w, ctx.actor, other).map_err(|_| CmdError::msg("No such destination."))?,
    };
    let plan = ops_plan(ctx, Vec::new(), vec![CustodyOp::Withdraw { object, vault, to }]);
    let outcome = api::post(ctx, &plan);
    finish_vault(ctx, vault, outcome, "Withdrawn")
}

fn vault_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_vault_id(ctx.args.lhs()).ok_or_else(|| CmdError::msg("No such vault."))?;
    if !may_view_vault(ctx.w, ctx.actor, id) {
        return Err(CmdError::msg("No such vault."));
    }
    let (header, backing, providers) = {
        let (v, providers) = {
            let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No such vault."))?;
            let v = store.snapshot().vault(id).ok_or_else(|| CmdError::msg("No such vault."))?.clone();
            (v, store.providers())
        };
        (
            format!(
                "vault #{} \"{}\" — owner {} [{:?}]",
                v.id,
                v.name,
                render_principal(ctx, &v.owner),
                v.status
            ),
            v.backing.clone(),
            providers,
        )
    };
    // Contents are a LIVE provider query, never a stored list — and contents are not
    // title (D5).
    let contents = crate::custody::resolve(&providers, ctx.w, &backing, None)
        .ok()
        .and_then(|p| p.contents(ctx.w, &backing));
    let mut lines = vec![header];
    if let Some(o) = backing.object() {
        lines.push(format!("  backing #{} ({})", o.0, display_name(ctx.w, o)));
    }
    match contents {
        Some(cs) if cs.is_empty() => lines.push("  empty".to_string()),
        Some(cs) => {
            let extra = cs.len().saturating_sub(LIST_CAP);
            for c in cs.iter().take(LIST_CAP) {
                lines.push(format!("  holds #{} ({})", c.0, display_name(ctx.w, *c)));
            }
            if extra > 0 {
                lines.push(format!("  ({extra} more)"));
            }
        }
        None => lines.push("  contents unavailable (no custody provider)".to_string()),
    }
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn vault_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let ids: Vec<VaultId> = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No economy."))?;
        store.snapshot().vaults().map(|v| v.id).collect()
    };
    let rows: Vec<String> = ids
        .into_iter()
        .filter(|id| may_view_vault(ctx.w, ctx.actor, *id))
        .map(|id| {
            let v = {
                let store = ctx.w.module_state::<EconomyStore>().expect("economy store");
                store.snapshot().vault(id).expect("listed above").clone()
            };
            format!("#{} {} — owner {} [{:?}]", v.id, v.name, render_principal(ctx, &v.owner), v.status)
        })
        .collect();
    emit_list(ctx, rows, "No vaults.");
    Ok(())
}

/// Staff doc write (not journaled), like `@asset/unfreeze`.
fn vault_set_status(ctx: &mut Ctx<'_>, status: VaultStatus, verb: &str) -> Result<(), CmdError> {
    require_admin(ctx)?;
    let id = parse_vault_id(ctx.args.lhs()).ok_or_else(|| CmdError::msg("No such vault."))?;
    if ctx.w.module_state::<EconomyStore>().is_none_or(|s| s.snapshot().vault(id).is_none()) {
        return Err(CmdError::msg("No such vault."));
    }
    api::set_vault_status(ctx, id, status);
    info(ctx, format!("Vault #{id} {verb}."));
    ctx.gate_on_durable_ack();
    Ok(())
}

fn vault_close(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_vault_id(ctx.args.lhs()).ok_or_else(|| CmdError::msg("No such vault."))?;
    if !may_view_vault(ctx.w, ctx.actor, id) {
        return Err(CmdError::msg("No such vault."));
    }
    let (owner, backing, providers) = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No such vault."))?;
        let v = store.snapshot().vault(id).ok_or_else(|| CmdError::msg("No such vault."))?;
        (v.owner.clone(), v.backing.clone(), store.providers())
    };
    if !is_admin(ctx.w, ctx.actor) && owner != PrincipalId::Object(ctx.actor) {
        return Err(vault_refusal(ctx, id, "You may not close that vault."));
    }
    let empty = crate::custody::resolve(&providers, ctx.w, &backing, None)
        .ok()
        .and_then(|p| p.contents(ctx.w, &backing))
        .is_some_and(|c| c.is_empty());
    if !empty {
        return Err(vault_refusal(ctx, id, "That vault is not empty."));
    }
    api::set_vault_status(ctx, id, VaultStatus::Closed);
    info(ctx, format!("Vault #{id} closed."));
    ctx.gate_on_durable_ack();
    Ok(())
}
