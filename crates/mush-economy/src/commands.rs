//! The staff economy command surface (E2b). All money movement goes through the
//! validated [`crate::post`]/[`crate::reverse`] seam — there is no balance-set
//! command (a correction is an attributed balanced transaction). Per R-D2 all E2
//! reads are staff-gated; the player-facing economy is the later ruleset slices.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::output::display_name;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ObjId, Powers, Role, World, match_object, perm,
};
use mush_org::{PrincipalId, SystemPrincipalId};
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::api::{self, PostOutcome};
use crate::ids::{AccountId, InstrumentId, TransactionId};
use crate::plan::{EconomyPlan, RequestedLine};
use crate::record::{
    Account, AccountStatus, CreditPolicy, EconomicCause, IdempotencyKey, InstrumentDef,
    InstrumentKind, InstrumentStatus, NegativePolicy,
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
            help: "@econ/pay <from>=<to> <amount>          transfer between two accounts\n\
                   @econ/mint <sysacct>=<to> <amount>      mint from a system source\n\
                   @econ/reverse <txid>                    reverse a transaction\n\
                   @econ/audit                             recent journal entries"
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_econ),
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

fn econ_pay(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (from, to, amount) = two_accounts_and_amount(ctx, "Usage: @econ/pay <from>=<to> <amount>")?;
    // Non-staff may pay only FROM an account they own (the engine re-checks live).
    if !is_admin(ctx.w, ctx.actor) {
        let owns = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().account(from))
            .is_some_and(|a| a.owner == PrincipalId::Object(ctx.actor));
        if !owns {
            return Err(CmdError::msg("You may only pay from your own account."));
        }
    }
    let neg = amount.checked_neg().ok_or_else(|| CmdError::msg("Amount out of range."))?;
    let plan = EconomyPlan::new(
        cmd_key(ctx),
        PrincipalId::Object(ctx.actor),
        EconomicCause::Command { serial: 0 },
        vec![
            RequestedLine { account: from, delta: neg, memo: SmolStr::default() },
            RequestedLine { account: to, delta: amount, memo: SmolStr::default() },
        ],
    );
    let outcome = api::post(ctx, &plan);
    finish(ctx, outcome, "Paid")
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
        Err(e) => Err(CmdError::msg(e.to_string())),
    }
}
