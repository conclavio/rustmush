//! The staff economy command surface (E2b). All money movement goes through the
//! validated [`crate::post`]/[`crate::reverse`] seam — there is no balance-set
//! command (a correction is an attributed balanced transaction). Per R-D2 all E2
//! reads are staff-gated; the player-facing economy is the later ruleset slices.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::output::display_name;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ObjId, Powers, Role, World, match_object, perm,
};
use mush_org::{OrgScopeId, PrincipalId, SystemPrincipalId};
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::api::{self, PostOutcome};
use crate::ids::{AccountId, InstrumentId, TransactionId, VaultId};
use crate::plan::{AssetOp, CustodyOp, EconomyPlan, RequestedLine};
use crate::record::{
    Account, AccountStatus, CreditPolicy, CustodyRef, EconomicCause, Encumbrance, IdempotencyKey,
    InstrumentDef, InstrumentKind, InstrumentStatus, NegativePolicy, Vault, VaultStatus,
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
            switches: ["open", "system", "show", "list"].iter().copied().map(SmolStr::new).collect(),
            help: "@account/open <obj>=<instrument>       open a personal account\n\
                   @account/system <name>=<instrument>    register a system source/sink\n\
                   @account/show <id>                     inspect one account\n\
                   @account/list                          list accounts"
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_account),
    )?;
    reg.command(
        CommandSpec {
            name: "@econ".into(),
            shape: ArgShape::EqPair,
            switches: ["pay", "mint", "reverse", "audit"].iter().copied().map(SmolStr::new).collect(),
            help: "@econ/pay <from>=<to> <amount> [asset=<obj>] [from_vault=<id>|to_vault=<id>]\n\
                   \x20                                      transfer money, and optionally the\n\
                   \x20                                      title/custody of one asset, in ONE plan\n\
                   @econ/mint <sysacct>=<to> <amount>      mint from a system source\n\
                   @econ/reverse <txid>                    reverse a transaction\n\
                   @econ/audit                             recent journal entries\n\
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
        Err(e) => Err(asset_refusal(ctx, object, e)),
    }
}
fn finish_vault(
    ctx: &mut Ctx<'_>,
    vault: VaultId,
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
        Err(e) => Err(vault_refusal(ctx, vault, e)),
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
    require_admin(ctx)?;
    if ctx.switches.has("open") {
        account_open(ctx, false)
    } else if ctx.switches.has("system") {
        account_open(ctx, true)
    } else if ctx.switches.has("show") {
        account_show(ctx)
    } else if ctx.switches.has("list") {
        account_list(ctx)
    } else {
        Err(CmdError::msg("Usage: @account/open|system|show|list"))
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
    let (owner, name, credit) = if system {
        let raw = ctx.args.lhs().trim();
        let sys = SystemPrincipalId::new(raw).ok_or_else(|| CmdError::msg("Bad system principal name (use [a-z0-9_.-])."))?;
        (PrincipalId::System(sys), SmolStr::new(raw), CreditPolicy::unbounded())
    } else {
        let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
        (PrincipalId::Object(obj), SmolStr::new(display_name(ctx.w, obj)), CreditPolicy::forbid())
    };
    let id = ctx.w.module_state_mut::<EconomyStore>().expect("economy store").alloc_account();
    let account = Account {
        id,
        owner: owner.clone(),
        instrument: instrument.clone(),
        scope: None,
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
    info(ctx, format!("Opened account #{id} ({instrument}){}.", if system { " [system source/sink]" } else { "" }));
    Ok(())
}

fn account_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_account(ctx.args.lhs()).ok_or_else(|| CmdError::msg("Usage: @account/show <id>"))?;
    let lines: Vec<String> = {
        let store = ctx.w.module_state::<EconomyStore>().expect("economy store");
        let a = store.snapshot().account(id).ok_or_else(|| CmdError::msg("No such account."))?;
        let scale = store.snapshot().instrument(&a.instrument).map_or(0, |i| i.scale);
        vec![
            format!("account #{} ({}) owner {:?} [{:?}]", a.id, a.instrument, a.owner, a.status),
            format!("  balance {} (held {})", a.posted_balance.to_decimal_string(scale), a.held_amount.to_decimal_string(scale)),
            format!("  last journal seq {}, revision {}", a.last_journal_seq, a.revision),
        ]
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn account_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lines: Vec<String> = {
        let store = ctx.w.module_state::<EconomyStore>().expect("economy store");
        let frozen = if store.is_frozen() { " [ECONOMY FROZEN]" } else { "" };
        let mut v: Vec<String> = store
            .snapshot()
            .accounts()
            .map(|a| {
                let scale = store.snapshot().instrument(&a.instrument).map_or(0, |i| i.scale);
                format!("#{} {} {} = {} [{:?}]", a.id, a.name, a.instrument, a.posted_balance.to_decimal_string(scale), a.status)
            })
            .collect();
        if v.is_empty() {
            v.push("No accounts.".into());
        }
        v.push(format!("(store{frozen})"));
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
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
    } else {
        Err(CmdError::msg("Usage: @econ/pay|mint|reverse|audit"))
    }
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
    // Non-staff may pay only FROM an account they own (the engine re-checks live).
    let staff = is_admin(ctx.w, ctx.actor);
    if !staff {
        let owns = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().account(from))
            .is_some_and(|a| a.owner == PrincipalId::Object(ctx.actor));
        if !owns {
            return Err(CmdError::msg("You may only pay from your own account."));
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
        Some(object) => finish_asset(ctx, object, outcome, "Paid"),
        None => finish(ctx, outcome, "Paid"),
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
    finish_asset(ctx, object, outcome, "Maintenance payer set")
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
    const USAGE: &str = "Usage: @vault/create <name>=container:<obj> [owner=<principal>] [scope=<id>]";
    let name = ctx.args.lhs().trim().to_string();
    if name.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let (rest, opts) = parse_opts(rhs, &["owner", "scope"]);
    let spec = rest.trim();
    let backing_obj = if let Some(o) = spec.strip_prefix("container:") {
        match_object(ctx.w, ctx.actor, o).map_err(|_| CmdError::msg("No such object."))?
    } else if spec.starts_with("interior:") {
        return Err(CmdError::msg(
            "Interior vaults are not available until the interior custody provider is wired.",
        ));
    } else {
        return Err(CmdError::msg(USAGE));
    };
    let backing = CustodyRef::Container(backing_obj);
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
        if let Some(p) = crate::custody::claimed_by(&providers, ctx.w, backing_obj) {
            return Err(CmdError::msg(format!(
                "That object is held by the {p} system and cannot back a container vault."
            )));
        }
        let usable = crate::custody::resolve(&providers, ctx.w, &backing, None)
            .ok()
            .and_then(|p| p.backing_valid(ctx.w, &backing))
            == Some(true);
        if !usable {
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
    Ok(())
}
