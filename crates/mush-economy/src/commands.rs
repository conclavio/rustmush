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
use crate::clock::{CadenceSource, Clocks, EconomicClock, EconomicDeadline, EconomicSchedule};
use crate::engine::AuthEval;
use crate::escalation::EscalationSubject;
use crate::ids::{
    AccountId, ApprovalRequestId, CadenceId, EscalationId, HoldId, InstrumentId, ObligationId,
    TransactionId, VaultId,
};
use crate::module::{EconomyStore, JOURNAL_LOG, PendingEconomyRead, PendingEconomyReadKind};
use crate::obligation::{
    GracePolicy, Obligation, ObligationCause, ObligationEvent, ObligationStatus, SuspendReason,
    SuspensionKey,
};
use crate::plan::{AssetOp, CustodyOp, EconomyPlan, HoldOp, NamedResource, RequestedLine};
use crate::record::{
    Account, AccountStatus, Approval, ApprovalRule, ApprovalStatus, ApprovalWindow, CreditPolicy,
    CustodyRef, Decision, EconomicCause, Encumbrance, HoldEffectKind, HoldPurpose, HoldStatus,
    IdempotencyKey, InstrumentDef, InstrumentDisplay, InstrumentKind, InstrumentStatus,
    NegativePolicy, SymbolPosition, Vault, VaultStatus, Verdict,
};

/// Register the economy commands.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "@instrument".into(),
            shape: ArgShape::EqPair,
            switches: ["register", "list"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
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
            switches: [
                "open", "system", "approval", "close", "unfreeze", "show", "list",
            ]
            .iter()
            .copied()
            .map(SmolStr::new)
            .collect(),
            help: "@account/open <obj>=<instrument>       open a personal account (staff)\n\
                   @account/open org:<org>[/<scope>]=<instrument>\n\
                   \x20                                     open an ORG wallet on a scope (staff)\n\
                   @account/system <name>=<instrument>    register a system source/sink (staff)\n\
                   @account/approval <id>=<threshold> <policy> [window=<total>/<secs>]\n\
                   \x20                                     high-value escalation (staff)\n\
                   \x20  policy = threshold:<n>[:<role>] | named:<n>:<obj>,<obj>… | none\n\
                   \x20  window=<total>/<secs> adds a rolling budget; window=off removes it\n\
                   @account/close <id>                    close an empty, active account\n\
                   @account/unfreeze <id>                 clear a recovery freeze (staff)\n\
                   @account/show <id>                     inspect one account\n\
                   @account/list                          list the accounts you may see\n\
                   A threshold is a PER-TRANSACTION escalation, not a budget: two payments\n\
                   under it both post without any approval. A window= IS the budget: the\n\
                   account's spend over one trailing period (placements counted, refunds\n\
                   never subtracted) needs the same approval once it would cross the total.\n\
                   A standing obligation's own period post is exempt from the window — it\n\
                   was consented to when the obligation was made — but not from the\n\
                   threshold."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_account),
    )?;
    reg.command(
        CommandSpec {
            name: "@econ".into(),
            shape: ArgShape::EqPair,
            switches: [
                "pay", "mint", "reverse", "audit", "history", "approve", "deny", "approvals",
                "unfreeze", "alerts", "ack", "cadence",
            ]
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
                   @econ/history <account> [n]             what touched an account you may see\n\
                   \x20                                      n is 1-20 (default 20)\n\
                   \x20                                      your own side in full; counterparties you\n\
                   \x20                                      may not see collapse to \"other accounts\",\n\
                   \x20                                      with no amounts and no count\n\
                   @econ/approve <id>   @econ/deny <id>    decide an approval request\n\
                   @econ/approvals                         requests awaiting your decision\n\
                   @econ/alerts                            open economy escalations (staff)\n\
                   @econ/ack <id>                          acknowledge one, and resume its subject\n\
                   \x20                                      where that is the recommendation (staff)\n\
                   @econ/cadence create <name> source=<real:<secs>|ic:<secs>|manual> [catchup=<n>]\n\
                   \x20                                      name a campaign clock (staff)\n\
                   @econ/cadence show <id>   @econ/cadence list\n\
                   @econ/cadence advance <id> [<n>]        step a MANUAL cadence (staff); a real- or\n\
                   \x20                                      IC-sourced clock is the sweep's to move\n\
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
                "writeoff", "unfreeze", "show", "list", "stolen", "catalog", "abandon",
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
                   @asset/stolen <obj>=<public|restricted|covert> [reason]\n\
                   \x20                                      report it stolen (the title owner, an\n\
                   \x20                                      insurer holding asset.insure on its\n\
                   \x20                                      compartment, or staff). It records a\n\
                   \x20                                      disputed claim; title never moves, and a\n\
                   \x20                                      later purchase is evidence, not guilt.\n\
                   @asset/catalog <obj>=<ruleset>:<code>|none    set the catalogue reference\n\
                   \x20                                      (staff). It is what a catalogue-keyed\n\
                   \x20                                      rule reads; changing it bumps the\n\
                   \x20                                      record's revision, so an outstanding\n\
                   \x20                                      quote priced against the old code\n\
                   \x20                                      lapses deterministically.\n\
                   @asset/unfreeze <obj>                         clear the recovery latch (staff)\n\
                   @asset/show <obj>\n\
                   @asset/list [owner=<principal>] [scope=<id>] [after=<obj>]  one bounded page\n\
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
            switches: [
                "create", "deposit", "withdraw", "show", "list", "freeze", "unfreeze", "close",
            ]
            .iter()
            .copied()
            .map(SmolStr::new)
            .collect(),
            help:
                "@vault/create <name>=container:<obj> [owner=<principal>] [scope=<id>]  (staff)\n\
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
    reg.command(
        CommandSpec {
            name: "@hold".into(),
            shape: ArgShape::EqPair,
            switches: ["place", "release", "capture", "show", "list"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@hold/place <account>=<amount> [for=<principal>] [expires=<secs|ic:<secs>>] [reason]\n\
                   \x20                                      reserve funds (debit authority; the\n\
                   \x20                                      account's approval rule applies)\n\
                   @hold/release <id>                     give a reservation back unspent\n\
                   @hold/capture <id>=<to account> <amount>  spend (part of) it; the rest is released\n\
                   @hold/show <id>    @hold/list [account=<id>]\n\
                   \x20                                      printed account ids are masked by your\n\
                   \x20                                      view standing, and a reservation held for a\n\
                   \x20                                      contract you are outside of shows the\n\
                   \x20                                      reservation and never the contract\n\
                   A hold makes money UNAVAILABLE without moving it: every debit is checked\n\
                   against posted minus held, so a reservation is a promise you cannot spend\n\
                   twice. expires= is a duration from now (ic: for the in-character clock); a\n\
                   hold with no deadline is released by hand, never aged out.\n\
                   Who may do what: the BENEFICIARY captures (that is what the reservation is\n\
                   for), the placer / beneficiary / anyone who may spend the account / staff\n\
                   release, and only the economy sweep expires."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_hold),
    )?;
    reg.command(
        CommandSpec {
            name: "@obligation".into(),
            shape: ArgShape::EqPair,
            switches: [
                "create", "show", "list", "suspend", "resume", "cancel", "advance", "repoint",
                "reinstate",
            ]
            .iter()
            .copied()
            .map(SmolStr::new)
            .collect(),
            help: "@obligation/create <debit acct>=<credit acct> <amount>\n\
                   \x20   [every=<secs|ic:<secs>|manual:<secs>>] [start=<secs>] [grace=<n>]\n\
                   \x20   [catchup=<n>] [notify]                       recurring debt (staff)\n\
                   @obligation/show <id>   @obligation/list\n\
                   \x20                                      printed account ids are masked by your\n\
                   \x20                                      view standing (\"an account\")\n\
                   @obligation/suspend <id>=<reason>   @obligation/resume <id>    (staff)\n\
                   @obligation/cancel <id>                                       (staff or creditor)\n\
                   @obligation/advance <id>               step a manual: clock one period\n\
                   \x20                                      (staff or creditor — advancing is the\n\
                   \x20                                      act of charging, so the debtor may not)\n\
                   @obligation/repoint <id>=<debit|credit> <account>             (staff)\n\
                   @obligation/reinstate <id>             Delinquent → Active, re-anchored to the\n\
                   \x20                                      first period due after now\n\
                   \x20                                      (staff or creditor)\n\
                   @econ/alerts   @econ/alerts <id>       open economy alerts; one alert's\n\
                   \x20                                      recommended commands (staff)\n\
                   The sweep attempts due periods on the obligation's own clock. A downtime or\n\
                   time-jump backlog is NEVER charged: the excess periods are forgiven with a\n\
                   recorded cause. A missed period enters grace; past the allowance it goes\n\
                   delinquent and leaves the due pool until someone reinstates it.\n\
                   every= omitted makes a ONE-SHOT; ic: runs on the in-character clock and\n\
                   manual: only moves when someone runs @obligation/advance. start= is a delay\n\
                   from now (default: one period out); grace= is 0-254. Routine settlement is\n\
                   silent unless you pass notify."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_obligation),
    )?;
    crate::contract_live::register(reg)?;
    Ok(())
}

// ---- helpers --------------------------------------------------------------

pub(crate) fn is_admin(w: &World, who: ObjId) -> bool {
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
pub(crate) fn refuse_if_frozen(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(EconomyStore::is_frozen)
    {
        return Err(CmdError::msg(
            "The economy is frozen; that is not available until staff clear it (@econ/unfreeze).",
        ));
    }
    Ok(())
}
pub(crate) fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "economy", msg.into());
}
pub fn parse_account(s: &str) -> Option<AccountId> {
    s.trim()
        .trim_start_matches('#')
        .parse::<u64>()
        .ok()
        .map(AccountId)
}
/// A unique idempotency key for an interactive command (each invocation is a fresh
/// intent). Keyed on the next transaction id, which is unique and never reused — so
/// two identical `@econ/pay`s both post (different keys), while an ambiguously-acked
/// retry of the SAME post (same key) dedups.
pub(crate) fn cmd_key(ctx: &Ctx<'_>) -> IdempotencyKey {
    let serial = ctx
        .w
        .module_state::<EconomyStore>()
        .map_or(0, EconomyStore::peek_next_transaction);
    IdempotencyKey::new(SmolStr::new_static("cmd"), SmolStr::new(serial.to_string()))
}

// ---- E3b1 shared parsing + the existence-hiding renderer ------------------

/// The row cap every `*/list` obeys (§4.5).
pub(crate) const LIST_CAP: usize = 20;

/// How many asset records one `@asset/list` page **visits** (E6b independent verify
/// fold, D10) — D-LBK-5's rule and `FENCE_PAGE_VISIT`'s number: a bounded read bounds
/// the records it touches, never the ones it matches, and the page names its own
/// cursor when there is more.
pub const ASSET_PAGE_VISIT: usize = 256;

/// Pull `key=value` options out of a free-form tail, returning the remaining words
/// and the recognized options (lower-cased keys). Unknown `k=v` words stay in the
/// remainder, so a purpose containing an `=` is not silently eaten.
pub fn parse_opts(s: &str, keys: &[&str]) -> (String, BTreeMap<String, String>) {
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
///
/// The `org:` arm goes through [`resolve_org_visible`], so a **hidden** organization
/// is unresolvable to an outsider by name *and* by id — every verb that takes a
/// principal therefore answers a hidden org exactly as it answers an unknown one
/// (E6a verify fold, D11).
pub fn parse_principal(ctx: &Ctx<'_>, s: &str) -> Option<PrincipalId> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("org:") {
        return resolve_org_visible(ctx, rest).map(PrincipalId::Organization);
    }
    if let Some(rest) = s.strip_prefix("sys:") {
        return SystemPrincipalId::new(rest).map(PrincipalId::System);
    }
    match_object(ctx.w, ctx.actor, s)
        .ok()
        .map(PrincipalId::Object)
}

/// Resolve `<id|name|alias>` to an organization the actor may be told **exists**
/// (E6a verify fold, D11).
///
/// `Organization.hidden` means "must not leak its existence to non-members" (org
/// §12.2), and a resolver that answered by id or by name regardless was an existence
/// oracle for every verb that takes an org: three distinguishable outcomes
/// ("no such organization" / "you may not spend it" / it worked) where there must be
/// one. A viewer who is staff, a member or a grantee resolves it; everybody else gets
/// `None`, which every caller renders exactly as an unknown org.
///
/// A **non-hidden** org resolves for anyone, as it always did — the rule is about
/// covert compartments, not about privacy in general.
#[must_use]
pub fn resolve_org_visible(ctx: &Ctx<'_>, s: &str) -> Option<mush_org::OrgId> {
    let id = resolve_org_any(ctx, s)?;
    let store = ctx.w.module_state::<mush_org::OrgStore>()?;
    let hidden = store.state().org(id).is_some_and(|o| o.hidden);
    if hidden && !may_see_org(ctx, id) {
        return None;
    }
    Some(id)
}

/// The raw lookup, blind to the hidden rule. Private: every player-facing caller
/// wants [`resolve_org_visible`].
fn resolve_org_any(ctx: &Ctx<'_>, s: &str) -> Option<mush_org::OrgId> {
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

pub(crate) fn parse_scope(s: &str) -> Option<OrgScopeId> {
    s.trim()
        .trim_start_matches('#')
        .parse::<u64>()
        .ok()
        .map(OrgScopeId)
}
pub(crate) fn parse_vault_id(s: &str) -> Option<VaultId> {
    s.trim()
        .trim_start_matches('#')
        .parse::<u64>()
        .ok()
        .map(VaultId)
}

/// The one player-facing amount render, over the live store (E6a step 1, E2-C16):
/// scale and symbol from `instrument`'s definition ([`crate::render_amount`]).
pub fn money(ctx: &Ctx<'_>, instrument: &InstrumentId, amount: Amount) -> String {
    ctx.w.module_state::<EconomyStore>().map_or_else(
        || amount.to_decimal_string(0),
        |s| crate::render::render_amount(s.snapshot(), instrument, amount),
    )
}

/// [`money`] in the instrument `account` holds — **behind the account's own view
/// gate** (E6a step 1 verify fold, D2).
///
/// Every caller renders an amount against an account id the actor may or may not be
/// allowed to know exists, so the gate is asked here, once, rather than at a dozen
/// call sites that would each be a chance to forget: an account the actor may not
/// view renders exactly as a nonexistent one does.
pub fn money_on(ctx: &Ctx<'_>, account: AccountId, amount: Amount) -> String {
    money_on_or(ctx, account, amount, None)
}

/// [`money_on`] with the **record's own instrument** as the fallback (M8): what an
/// unviewable or unresolvable account renders in, rather than raw minor at scale 0.
pub fn money_on_or(
    ctx: &Ctx<'_>,
    account: AccountId,
    amount: Amount,
    fallback: Option<&InstrumentId>,
) -> String {
    let visible = may_view_account(ctx.w, ctx.actor, account);
    ctx.w.module_state::<EconomyStore>().map_or_else(
        || amount.to_decimal_string(0),
        |s| crate::render::render_amount_on(s.snapshot(), account, amount, visible, fallback),
    )
}

/// Render a principal for the actor — **behind the hidden-org rule** (E6a step 1
/// verify fold, D7; 08f G3 brought forward).
///
/// `Organization.hidden` means "must not leak its existence to non-members" (org
/// §12.2), and this named one unconditionally. It always mattered a little and now
/// matters more: `contract_notice`'s `announce=yes` line renders the issuer to an
/// **audience**, which is by definition people outside the terms, so a covert
/// backer's name reached everyone who could see the run existed. A viewer who is
/// staff, a member, or a grantee of the org sees its name; everybody else sees
/// "an organization" — the same shape every other existence-hiding render in the
/// crate uses, and the id goes with the name (an id is a handle).
pub fn render_principal(ctx: &Ctx<'_>, p: &PrincipalId) -> String {
    match p {
        PrincipalId::Object(o) => format!("{} (#{})", display_name(ctx.w, *o), o.0),
        PrincipalId::Organization(o) => {
            let Some(store) = ctx.w.module_state::<mush_org::OrgStore>() else {
                return format!("org:{o}");
            };
            let state = store.state();
            let Some(org) = state.org(*o) else {
                return format!("org:{o}");
            };
            if org.hidden && !may_see_org(ctx, *o) {
                return "an organization".to_string();
            }
            format!("org:{} (#{o})", org.name)
        }
        PrincipalId::System(s) => format!("sys:{s}"),
    }
}

/// Whether the actor may be told a **hidden** organization exists (D7): staff, a
/// member, or a grantee — the same two rosters
/// [`crate::principal_holders`] walks, without the capability filter, because
/// "may know it exists" is a wider question than "may spend its money".
fn may_see_org(ctx: &Ctx<'_>, org: mush_org::OrgId) -> bool {
    if api::is_staff(ctx.w, ctx.actor) {
        return true;
    }
    let Some(store) = ctx.w.module_state::<mush_org::OrgStore>() else {
        return false;
    };
    let state = store.state();
    state.members_in(org).any(|m| m.member == ctx.actor)
        || state.grants_in(org).any(|g| g.grantee == ctx.actor)
}

/// §4.5 view standing over an asset: staff, its title owner, its assignee, or a
/// holder of an `asset.*` capability on its (live) compartment.
pub fn may_view_asset(w: &World, actor: ObjId, object: ObjId) -> bool {
    if api::is_staff(w, actor) {
        return true;
    }
    let Some(store) = w.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(a) = store.snapshot().asset(object) else {
        return false;
    };
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
    let Some(store) = w.module_state::<EconomyStore>() else {
        return false;
    };
    let snap = store.snapshot();
    let Some(a) = snap.account(id) else {
        return false;
    };
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
    snap.approvals().any(|r| {
        r.status == ApprovalStatus::Pending && now < r.expires_at && r.accounts.contains(&id)
    })
}

/// Render an account id **through the view gate** — the one masking rule the whole
/// disclosure filter uses (E4c, §6.1 item 5).
///
/// It is deliberately a renderer over [`may_view_account`] and not a second
/// predicate: `@econ/history`, `@contract/show` and `@hold/show` all show account
/// ids belonging to somebody else, and three hand-rolled "if I can see it" branches
/// would be three chances for one of them to drift open. `hidden` is the caller's
/// noun for the same fact — "an account" in a contract's terms, "another account"
/// for the counterparty of an entry the viewer is reading their own side of.
pub fn render_account(ctx: &Ctx<'_>, id: AccountId, noun: &str) -> String {
    if may_view_account(ctx.w, ctx.actor, id) {
        format!("account #{id}")
    } else {
        noun.to_string()
    }
}

/// The former name of [`render_account`], kept as the one-line alias so no caller
/// outside this crate has to move (E6b, B1).
#[must_use]
pub fn mask_account(ctx: &Ctx<'_>, id: AccountId, hidden: &str) -> String {
    render_account(ctx, id, hidden)
}

// ---- E6b: B1 — one masking renderer ---------------------------------------

/// The **one token** every masked counterparty on one journal entry collapses into
/// (E4c verify fold, D4).
///
/// One `another account` per hidden line published the arity of the split — a
/// runner could count the other runners — and with the viewer's own side and the
/// total in hand a two-hidden-line entry was solvable by subtraction. One token
/// says the same true thing and counts nothing.
pub const MASKED_ACCOUNTS: &str = "other accounts";

/// The history renderer's half of [`render_account`]'s predicate (B1): `Some(line)`
/// when the viewer may see this account, `None` when the caller must collapse it
/// into [`MASKED_ACCOUNTS`].
///
/// It is a *second helper on the same predicate* rather than a second predicate,
/// which is the whole point of the fold: an inline `if may_view_account(..)` at a
/// render site is one more place that can drift open.
#[must_use]
pub fn render_account_entry(
    ctx: &Ctx<'_>,
    id: AccountId,
    instrument: &InstrumentId,
    amount: Amount,
) -> Option<String> {
    may_view_account(ctx.w, ctx.actor, id)
        .then(|| format!("account #{id} {}", money(ctx, instrument, amount)))
}

/// Render an organization for a **listing** — the name iff the viewer may be told
/// it exists, else "an organization" (08f §4.3 as amended, G3/D7).
///
/// A thin named wrapper over [`render_principal`] and [`may_see_org`], not new
/// logic: E6a step 1's fold D7 already built exactly that pair. The economy owns
/// the *render*; a ruleset owns the *absence* — a hidden vendor's listings are
/// simply not in its list for a non-member, never "No such".
#[must_use]
pub fn render_org_listed(ctx: &Ctx<'_>, org: mush_org::OrgId) -> String {
    render_principal(ctx, &PrincipalId::Organization(org))
}

/// Whether the actor may be told a **hidden** organization exists — the predicate
/// behind [`render_org_listed`], made public in E6b so a ruleset can omit a hidden
/// vendor's listings rather than refuse them (G3, scenario 12).
#[must_use]
pub fn may_list_org(ctx: &Ctx<'_>, org: mush_org::OrgId) -> bool {
    let Some(store) = ctx.w.module_state::<mush_org::OrgStore>() else {
        return false;
    };
    let hidden = store.state().org(org).is_some_and(|o| o.hidden);
    !hidden || may_see_org(ctx, org)
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
pub(crate) fn hidden_text(ctx: &Ctx<'_>, resource: NamedResource) -> Option<&'static str> {
    match resource {
        NamedResource::Account(id) => {
            (!may_view_account(ctx.w, ctx.actor, id)).then_some("No such account.")
        }
        NamedResource::Asset(o) => {
            (!may_view_asset(ctx.w, ctx.actor, o)).then_some("No such asset.")
        }
        NamedResource::Vault(v) => {
            (!may_view_vault(ctx.w, ctx.actor, v)).then_some("No such vault.")
        }
        NamedResource::Hold(h) => (!may_view_hold(ctx.w, ctx.actor, h)).then_some("No such hold."),
        NamedResource::Obligation(o) => {
            (!may_view_obligation(ctx.w, ctx.actor, o)).then_some("No such obligation.")
        }
        NamedResource::Contract(c) => {
            (!crate::contract_live::may_view_contract(ctx.w, ctx.actor, c))
                .then_some("No such contract.")
        }
    }
}

/// §4.5 view standing over a **hold**: staff, anyone who may view the account it
/// reserves against, the principal who placed it, or **anyone who acts for its
/// beneficiary**.
///
/// The beneficiary clause is the load-bearing one: escrow is a promise made TO
/// someone, and a payee who cannot see the reservation cannot tell whether they have
/// been paid a promise or a story.
///
/// It asks [`api::WorldAuth::acts_for`] — the *same* predicate that decides who may
/// capture — rather than an identity match (E4b1 verify fold, A2). An identity match
/// is wrong in exactly the cases escrow is for: a hold made out to an **organization**
/// was invisible to every person in it, and a hold made out to a **Thing** (a shop
/// counter, a guild strongbox) was invisible to the player who controls it. Both
/// could capture in principle, and every capture verb checks viewability first, so
/// the promise was unspendable as well as unseeable. Tying the two to one predicate
/// means seeing a reservation and being able to spend it can never drift apart.
#[must_use]
pub fn may_view_hold(w: &World, actor: ObjId, id: HoldId) -> bool {
    if api::is_staff(w, actor) {
        return true;
    }
    let (placed_by, beneficiary, account) = {
        let Some(store) = w.module_state::<EconomyStore>() else {
            return false;
        };
        let Some(h) = store.snapshot().hold(id) else {
            return false;
        };
        (h.placed_by.clone(), h.beneficiary.clone(), h.account)
    };
    let me = PrincipalId::Object(actor);
    if placed_by == me {
        return true;
    }
    let auth = api::WorldAuth { w };
    if let Some(b) = &beneficiary
        && auth.acts_for(&me, b)
    {
        return true;
    }
    may_view_account(w, actor, account)
}

/// §4.5 view standing over an **obligation**: staff, anyone who **acts for** either
/// party, or anyone who may view either of its two accounts (a landlord watching the
/// credit side, a tenant watching the debit side).
///
/// The party test is [`api::WorldAuth::acts_for`] and not an identity match (E6a
/// step 1 verify fold, D1) — the same A2 shape [`may_view_hold`] and the contract
/// seams already use. An identity match was wrong in exactly the case E6a is for: an
/// **organization** debtor was "a party" no person matched, so an officer holding
/// `economy.account.post` on the org's root — who may spend its money, and whom
/// `principal_holders` tells about every transition — could be told
/// "Obligation #12 is delinquent." by the org inbox and then answered "No such
/// obligation." by `@obligation/show`. The inbox and the gate now ask one question.
#[must_use]
pub fn may_view_obligation(w: &World, actor: ObjId, id: ObligationId) -> bool {
    if api::is_staff(w, actor) {
        return true;
    }
    let (debtor, creditor, debit, credit) = {
        let Some(store) = w.module_state::<EconomyStore>() else {
            return false;
        };
        let Some(o) = store.snapshot().obligation(id) else {
            return false;
        };
        (
            o.debtor.clone(),
            o.creditor.clone(),
            o.debit_account,
            o.credit_account,
        )
    };
    let me = PrincipalId::Object(actor);
    let auth = api::WorldAuth { w };
    auth.acts_for(&me, &debtor)
        || auth.acts_for(&me, &creditor)
        || may_view_account(w, actor, debit)
        || may_view_account(w, actor, credit)
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
pub fn refuse(
    ctx: &mut Ctx<'_>,
    subject: NamedResource,
    also: &[NamedResource],
    e: &crate::plan::EconomyError,
) -> CmdError {
    let named = e.named_resources();
    let extra: &[NamedResource] = if e.concerns_named_subjects() {
        also
    } else {
        &[]
    };
    let mut hidden: Option<String> = None;
    for r in std::iter::once(subject)
        .chain(named)
        .chain(extra.iter().copied())
    {
        if let Some(text) = hidden_text(ctx, r) {
            hidden = Some(text.to_string());
            break;
        }
    }
    note_refusal(ctx, hidden.is_some());
    CmdError::msg(hidden.unwrap_or_else(|| e.to_string()))
}

// ---- E6b: B5 — C16, bounded per-actor refusal telemetry -------------------

/// Count one refusal against the actor (E6b, C16; 08g §11 fix-during).
///
/// The existence-hiding renderer is where a probe becomes indistinguishable from a
/// typo, which is exactly the point of it — and exactly why staff need to be able
/// to see the *shape* of one. This is the whole mechanism: a bounded, RAM-only
/// per-actor count, split into hidden and ordinary refusals, surfaced by
/// `@econ/alerts` and read by nothing that decides anything.
pub fn note_refusal(ctx: &mut Ctx<'_>, hidden: bool) {
    let actor = ctx.actor;
    let now = ctx.w.now();
    if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
        store.note_refusal(actor, hidden, now);
    }
}

/// Whether this resource renders as "No such …" for this actor — the predicate half
/// of [`hidden_text`], for callers that are shaping a *message* rather than a refusal
/// (E4b2 verify fold, A6).
pub fn is_hidden(ctx: &Ctx<'_>, resource: NamedResource) -> bool {
    hidden_text(ctx, resource).is_some()
}

/// [`refuse`] for a refusal that is not an [`crate::EconomyError`] (E4b2 verify fold,
/// A2): the same hiding loop over an explicit list of resources, so a contract
/// refusal and a money refusal disclose exactly the same amount.
pub fn refuse_named(
    ctx: &mut Ctx<'_>,
    subject: NamedResource,
    named: impl IntoIterator<Item = NamedResource>,
    detail: String,
) -> CmdError {
    let mut hidden: Option<String> = None;
    for r in std::iter::once(subject).chain(named) {
        if let Some(text) = hidden_text(ctx, r) {
            hidden = Some(text.to_string());
            break;
        }
    }
    note_refusal(ctx, hidden.is_some());
    CmdError::msg(hidden.unwrap_or(detail))
}

/// §4.5 view standing over a vault: staff, its owner, or `vault.view` on its scope.
///
/// Exported (E7c2 fold C8, 08i answer 16's owner-exported `pub fn` seam) so a
/// neutral quantity authority bound to E3 custody endpoints asks **this**
/// question instead of keeping a second, disagreeing copy of it.
#[must_use]
pub fn may_view_vault(w: &World, actor: ObjId, id: VaultId) -> bool {
    if api::is_staff(w, actor) {
        return true;
    }
    let Some(store) = w.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(v) = store.snapshot().vault(id) else {
        return false;
    };
    if v.owner == PrincipalId::Object(actor) {
        return true;
    }
    let Some(scope) = v.scope else { return false };
    api::scope_coherent(w, Some(scope), &v.owner)
        && api::holds_capability(w, actor, "vault.view", scope)
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
fn ops_plan(
    ctx: &mut Ctx<'_>,
    asset_ops: Vec<AssetOp>,
    custody_ops: Vec<CustodyOp>,
) -> EconomyPlan {
    let mut p = EconomyPlan::new(
        cmd_key(ctx),
        PrincipalId::Object(ctx.actor),
        EconomicCause::Command {
            serial: ctx.w.next_correlation_id(),
        },
        Vec::new(),
    );
    p.asset_ops = asset_ops;
    p.custody_ops = custody_ops;
    p
}

// ---- E6b: B9 — the programmatic deposit seam ------------------------------

/// **Deposit an object into a vault** under the actor's own authority (E6b, B9) —
/// `@vault/deposit` without the command.
///
/// The sixth programmatic seam, and it exists for the same reason `cancel_contract`
/// did: a ruleset must never dispatch a `@vault` command from inside a handler, and
/// E6b's fence has to hand custody over on the seller's own standing. Every gate the
/// command applies applies here — the actor's custody standing over the object, the
/// vault's liveness, the destination rules, the whole plan validated and posted by
/// [`crate::post`] — because it *is* the same plan.
///
/// # Errors
///
/// Every [`crate::EconomyError`] a custody post can produce. Render it with
/// [`refuse`] against [`NamedResource::Vault`] so an unviewable vault reads exactly
/// as a nonexistent one.
pub fn deposit_into_vault(
    ctx: &mut Ctx<'_>,
    object: ObjId,
    vault: VaultId,
) -> Result<PostOutcome, crate::plan::EconomyError> {
    let plan = ops_plan(ctx, Vec::new(), vec![CustodyOp::Deposit { object, vault }]);
    api::post(ctx, &plan)
}

/// Parse a `<ruleset>:<code>` catalog reference (E6b). Both halves are bounded and
/// non-empty; the economy stores it opaquely and never interprets either.
#[must_use]
pub fn parse_catalog(s: &str) -> Option<crate::record::CatalogRef> {
    let (ruleset, code) = s.trim().split_once(':')?;
    let (ruleset, code) = (ruleset.trim(), code.trim());
    if ruleset.is_empty()
        || code.is_empty()
        || ruleset.len() > MAX_CATALOG_PART_BYTES
        || code.len() > MAX_CATALOG_PART_BYTES
        || ruleset.chars().any(char::is_whitespace)
        || code.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some(crate::record::CatalogRef {
        ruleset: SmolStr::new(ruleset),
        code: SmolStr::new(code),
    })
}

/// The longest either half of a [`crate::CatalogRef`] may be (E6b).
pub const MAX_CATALOG_PART_BYTES: usize = 64;

/// Resolve `<obj>` for an asset verb. A name that matches nothing renders as the
/// nonexistent-asset text (never "I don't see that here", which would leak).
pub(crate) fn asset_target(ctx: &Ctx<'_>, s: &str) -> Result<ObjId, CmdError> {
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
pub(crate) fn emit_list(ctx: &mut Ctx<'_>, mut rows: Vec<String>, empty: &str) {
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
    const USAGE: &str =
        "Usage: @instrument/register <code>=<scale> [display=<symbol>:prefix|suffix]";
    let code = ctx.args.lhs().trim();
    let id = InstrumentId::new(code)
        .ok_or_else(|| CmdError::msg("Bad instrument code (use [a-z0-9_.-])."))?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let (rest, opts) = parse_opts(rhs, &["display"]);
    let scale: u8 = rest.trim().parse().map_err(|_| CmdError::msg(USAGE))?;
    if scale > crate::MAX_SCALE {
        return Err(CmdError::msg(format!(
            "Scale too large (max {}).",
            crate::MAX_SCALE
        )));
    }
    // E6a step 1 (E2-C16): `display=<symbol>:prefix|suffix` — `¥:prefix` renders
    // `¥1000`, `€:suffix` renders `12.50€`.
    let display = match opts.get("display") {
        None => None,
        Some(spec) => {
            let (symbol, position) = spec.rsplit_once(':').ok_or_else(|| CmdError::msg(USAGE))?;
            let position = match position.trim().to_ascii_lowercase().as_str() {
                "prefix" => SymbolPosition::Prefix,
                "suffix" => SymbolPosition::Suffix,
                _ => return Err(CmdError::msg(USAGE)),
            };
            Some(InstrumentDisplay::new(symbol, position).ok_or_else(|| {
                CmdError::msg(format!(
                    "A display symbol is 1-{} bytes with no whitespace.",
                    crate::record::MAX_SYMBOL_BYTES
                ))
            })?)
        }
    };
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(|s| s.snapshot().instrument(&id).is_some())
    {
        return Err(CmdError::msg("That instrument is already registered."));
    }
    let rendered = display.as_ref().map(|d| match d.position {
        SymbolPosition::Prefix => format!("{}<amount>", d.symbol),
        SymbolPosition::Suffix => format!("<amount>{}", d.symbol),
    });
    let def = InstrumentDef {
        id: id.clone(),
        ruleset: SmolStr::new_static("staff"),
        code: SmolStr::new(code),
        kind: InstrumentKind::Currency,
        scale,
        transferable: true,
        negative_policy: NegativePolicy::SystemOnly,
        status: InstrumentStatus::Active,
        display,
        revision: 1,
    };
    api::register_instrument(ctx, def).map_err(|e| CmdError::msg(e.to_string()))?;
    match rendered {
        Some(r) => info(
            ctx,
            format!("Registered instrument {id} (scale {scale}, renders as {r})."),
        ),
        None => info(ctx, format!("Registered instrument {id} (scale {scale}).")),
    }
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
        Err(CmdError::msg(
            "Usage: @account/open|system|approval|close|unfreeze|show|list",
        ))
    }
}

fn account_open(ctx: &mut Ctx<'_>, system: bool) -> Result<(), CmdError> {
    let instrument_code = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Usage: @account/open <obj>=<instrument>"))?;
    let instrument =
        InstrumentId::new(instrument_code).ok_or_else(|| CmdError::msg("Bad instrument code."))?;
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_none_or(|s| s.snapshot().instrument(&instrument).is_none())
    {
        return Err(CmdError::msg("No such instrument (register it first)."));
    }
    let lhs = ctx.args.lhs().trim().to_string();
    let (owner, name, credit, scope) = if system {
        let sys = SystemPrincipalId::new(&lhs)
            .ok_or_else(|| CmdError::msg("Bad system principal name (use [a-z0-9_.-])."))?;
        (
            PrincipalId::System(sys),
            SmolStr::new(&lhs),
            CreditPolicy::unbounded(),
            None,
        )
    } else if let Some(spec) = lhs.strip_prefix("org:") {
        // `org:<org>[/<scope>]` — the divisional wallet (§4.6). The compartment is
        // the policy anchor: everything about who may spend from this account is a
        // question about `scope`, answered live by org.
        let (org_s, scope_s) = match spec.split_once('/') {
            Some((o, s)) => (o, Some(s)),
            None => (spec, None),
        };
        let org = resolve_org_visible(ctx, org_s).ok_or_else(|| CmdError::msg("No such org."))?;
        let (scope, label) = {
            let store = ctx
                .w
                .module_state::<mush_org::OrgStore>()
                .ok_or_else(|| CmdError::msg("No organizations."))?;
            let state = store.state();
            let o = state.org(org).expect("resolved org exists");
            let sid = match scope_s {
                None => o.root_scope,
                Some(s) => parse_scope(s).ok_or_else(|| CmdError::msg("Bad scope id."))?,
            };
            let sc = state
                .scope(sid)
                .ok_or_else(|| CmdError::msg("No such scope."))?;
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
        (
            owner,
            SmolStr::new(label),
            CreditPolicy::forbid(),
            Some(scope),
        )
    } else {
        let obj = match_object(ctx.w, ctx.actor, &lhs)?;
        (
            PrincipalId::Object(obj),
            SmolStr::new(display_name(ctx.w, obj)),
            CreditPolicy::forbid(),
            None,
        )
    };
    // B2: the allocator refuses at the ceiling instead of overflowing.
    let id = ctx
        .w
        .module_state_mut::<EconomyStore>()
        .expect("economy store")
        .alloc_account()
        .ok_or_else(|| {
            CmdError::msg("The account-id allocator is exhausted; nothing was opened.")
        })?;
    let account = Account {
        id,
        owner: owner.clone(),
        instrument: instrument.clone(),
        scope,
        name,
        status: AccountStatus::Active,
        posted_balance: Amount::ZERO,
        held_amount: Amount::ZERO,
        last_hold_seq: 0,
        credit,
        last_journal_seq: 0,
        revision: 1,
        // High-value escalation is opt-in per account (E3b2 sets it).
        approval: None,
    };
    api::open_account(ctx, account);
    let where_ = scope.map_or_else(String::new, |s| format!(" on scope #{s}"));
    info(
        ctx,
        format!(
            "Opened account #{id} ({instrument}){where_}{}.",
            if system { " [system source/sink]" } else { "" }
        ),
    );
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
    let a = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No such account."))?;
        store
            .snapshot()
            .account(id)
            .ok_or_else(|| CmdError::msg("No such account."))?
            .clone()
    };
    let mut lines = vec![
        format!(
            "account #{} \"{}\" ({}) [{:?}]",
            a.id, a.name, a.instrument, a.status
        ),
        format!("  balance {}", money(ctx, &a.instrument, a.posted_balance)),
    ];
    if let Some(rule) = &a.approval {
        lines.push(format!(
            "  approval above {}: {}",
            money(ctx, &a.instrument, rule.threshold),
            render_policy(ctx, &rule.policy)
        ));
        if let Some(w) = &rule.window {
            lines.push(format!(
                "  rolling window: {} per {} s",
                money(ctx, &a.instrument, w.total),
                w.period_secs
            ));
        }
    }
    if staff {
        lines.push(format!("  owner {}", render_principal(ctx, &a.owner)));
        if let Some(s) = a.scope {
            lines.push(format!("  scope #{s}"));
        }
        lines.push(format!(
            "  held {}",
            money(ctx, &a.instrument, a.held_amount)
        ));
        lines.push(format!(
            "  last journal seq {}, revision {}",
            a.last_journal_seq, a.revision
        ));
    }
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn render_policy(ctx: &Ctx<'_>, p: &ApprovalPolicy) -> String {
    match p {
        ApprovalPolicy::OneAuthorizedActor => "one authorized actor (inert)".to_string(),
        ApprovalPolicy::Threshold {
            approvals,
            from_role,
        } => match from_role {
            None => format!("{approvals} approval(s)"),
            Some(r) => {
                let name = ctx
                    .w
                    .module_state::<mush_org::OrgStore>()
                    .and_then(|s| s.state().role(*r).map(|x| x.name.to_string()));
                format!(
                    "{approvals} approval(s) from role {}",
                    name.unwrap_or_else(|| format!("#{r}"))
                )
            }
        },
        ApprovalPolicy::NamedSet { approvals, actors } => format!(
            "{approvals} approval(s) from {}",
            actors
                .iter()
                .map(|o| format!("#{}", o.0))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// `@account/list` — every account the actor may VIEW, capped at 20 (§4.5). An
/// actor with no standing sees an empty list, never a count of what is hidden.
fn account_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let staff = is_admin(ctx.w, ctx.actor);
    let ids: Vec<AccountId> = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        store.snapshot().accounts().map(|a| a.id).collect()
    };
    let rows: Vec<String> = ids
        .into_iter()
        .filter(|id| may_view_account(ctx.w, ctx.actor, *id))
        .map(|id| {
            let store = ctx.w.module_state::<EconomyStore>().expect("economy store");
            let a = store.snapshot().account(id).expect("listed above");
            format!(
                "#{} {} {} = {} [{:?}]{}",
                a.id,
                a.name,
                a.instrument,
                money(ctx, &a.instrument, a.posted_balance),
                a.status,
                if a.approval.is_some() {
                    " (approval rule)"
                } else {
                    ""
                }
            )
        })
        .collect();
    emit_list(ctx, rows, "No accounts.");
    if staff
        && ctx
            .w
            .module_state::<EconomyStore>()
            .is_some_and(EconomyStore::is_frozen)
    {
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
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No such account."))?;
        store
            .snapshot()
            .account(id)
            .ok_or_else(|| CmdError::msg("No such account."))?
            .clone()
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
        return Err(account_refusal(
            ctx,
            id,
            "That account still holds a balance.",
        ));
    }
    // Invariant 29 (E4a): the E3 close path was written when `held_amount` was
    // permanently zero. A closed account takes no posting, so closing one that still
    // carries a live reservation would trap escrow forever, and closing one an
    // obligation names would make every future period an unfixable refusal. Both are
    // checked as their own question, not inferred from the balance: a hold whose
    // account drifted to zero, or a live obligation on an empty account, would slip
    // through the amount test above.
    {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        let snap = store.snapshot();
        if snap.holds_of(id).any(|h| h.status.is_held()) {
            return Err(account_refusal(
                ctx,
                id,
                "That account still carries an active hold; release or capture it first.",
            ));
        }
        // E4b1 verify fold, A5: **any** obligation that is not finished, not just one
        // in the due pool. `in_due_pool()` is `Active | Grace`, so a `Delinquent`
        // obligation — which `@obligation/reinstate` puts straight back into the pool
        // — and a `Suspended` one — which `@econ/ack` resumes — both let the account
        // close underneath them, and the reinstatement then had nowhere to settle.
        // Only `Cancelled` and `Satisfied` are genuinely over.
        if let Some(o) = snap.obligations().find(|o| {
            (o.debit_account == id || o.credit_account == id)
                && !matches!(
                    o.status,
                    ObligationStatus::Cancelled | ObligationStatus::Satisfied
                )
        }) {
            return Err(account_refusal(
                ctx,
                id,
                format!(
                    "Obligation #{} still names that account ({}); cancel or repoint it first.",
                    o.id,
                    render_status(o)
                ),
            ));
        }
    }
    api::set_account_status(ctx, id, AccountStatus::Closed);
    info(ctx, format!("Account #{id} closed."));
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `@account/unfreeze <id>` — staff recovery (E2 shipped no unfreeze). A doc write,
/// never journaled, and deliberately no `revision` bump (see `set_account_status`).
fn account_unfreeze(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_account(ctx.args.lhs())
        .ok_or_else(|| CmdError::msg("Usage: @account/unfreeze <id>"))?;
    let status = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No such account."))?;
        store
            .snapshot()
            .account(id)
            .ok_or_else(|| CmdError::msg("No such account."))?
            .status
    };
    if status != AccountStatus::Frozen {
        return Err(CmdError::msg("That account is not frozen."));
    }
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(EconomyStore::is_frozen)
    {
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
                         named:<n>:<obj>,<obj>… [window=<total>/<secs>|off] | \
                         @account/approval <id>=window=<total>/<secs>|off | \
                         @account/approval <id>=none";
    let id = parse_account(ctx.args.lhs()).ok_or_else(|| CmdError::msg(USAGE))?;
    let account = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No such account."))?;
        store
            .snapshot()
            .account(id)
            .ok_or_else(|| CmdError::msg("No such account."))?
            .clone()
    };
    let rhs = ctx
        .args
        .rhs()
        .map(str::trim)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    if rhs.eq_ignore_ascii_case("none") {
        api::set_account_approval(ctx, id, None);
        info(ctx, format!("Cleared the approval rule on account #{id}."));
        ctx.gate_on_durable_ack();
        return Ok(());
    }
    // E6a step 1 (D-E6-21): the rolling window beside the threshold. `window=off`
    // restores threshold-only; a bare `window=` on an existing rule changes only the
    // window and leaves the threshold and policy as they are.
    let (rest, opts) = parse_opts(rhs, &["window"]);
    let window_spec: Option<Option<ApprovalWindow>> = match opts.get("window") {
        None => None,
        Some(spec) => Some(parse_window(spec)?),
    };
    let rest = rest.trim();
    if rest.is_empty() {
        let Some(window) = window_spec else {
            return Err(CmdError::msg(USAGE));
        };
        let Some(mut rule) = account.approval.clone() else {
            return Err(CmdError::msg(format!(
                "Account #{id} has no approval rule; set one (a threshold and a policy) before \
                 attaching a window to it."
            )));
        };
        rule.window = window.clone();
        api::set_account_approval(ctx, id, Some(rule));
        let notice = window_notice(ctx, &account, window.as_ref());
        info(ctx, notice);
        ctx.gate_on_durable_ack();
        return Ok(());
    }
    let mut words = rest.split_whitespace();
    let threshold = words
        .next()
        .and_then(parse_amount)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    if threshold.is_negative() {
        return Err(CmdError::msg("A threshold must not be negative."));
    }
    let spec = words
        .next()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .to_string();
    if spec.eq_ignore_ascii_case("none") {
        api::set_account_approval(ctx, id, None);
        info(ctx, format!("Cleared the approval rule on account #{id}."));
        ctx.gate_on_durable_ack();
        return Ok(());
    }
    let policy = parse_policy(ctx, &account, &spec)?;
    // A fresh rule carries no window unless one was named; an existing rule keeps
    // its window when the words re-state only the threshold and the policy.
    let window = match window_spec {
        Some(w) => w,
        None => account.approval.as_ref().and_then(|r| r.window.clone()),
    };
    api::set_account_approval(
        ctx,
        id,
        Some(ApprovalRule {
            threshold,
            policy: policy.clone(),
            window: window.clone(),
        }),
    );
    let rendered = render_policy(ctx, &policy);
    let threshold_text = money(ctx, &account.instrument, threshold);
    info(
        ctx,
        format!("Account #{id}: a net debit above {threshold_text} now needs {rendered}."),
    );
    match &window {
        Some(w) => {
            let notice = window_notice(ctx, &account, Some(w));
            info(ctx, notice);
        }
        None => info(
            ctx,
            "That is a per-transaction escalation, not a budget.".to_string(),
        ),
    }
    // The rule is read live at every post, so the confirmation must not outrun the
    // doc that carries it (non-strict: policy, not money).
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `window=<total>/<secs>` → `Some(window)`; `window=off` → `None`.
fn parse_window(spec: &str) -> Result<Option<ApprovalWindow>, CmdError> {
    const USAGE: &str = "window= takes <total>/<seconds> (the most an account may spend inside \
                         one trailing period, and the period), or `off`.";
    if spec.trim().eq_ignore_ascii_case("off") {
        return Ok(None);
    }
    let (total, secs) = spec.split_once('/').ok_or_else(|| CmdError::msg(USAGE))?;
    let total = parse_amount(total).ok_or_else(|| CmdError::msg(USAGE))?;
    let secs = secs
        .trim()
        .parse::<i64>()
        .map_err(|_| CmdError::msg(USAGE))?;
    ApprovalWindow::new(total, secs).map(Some).ok_or_else(|| {
        CmdError::msg(format!(
            "A window needs a non-negative total and a period of 1-{} seconds.",
            crate::record::MAX_APPROVAL_WINDOW_SECS
        ))
    })
}

/// The confirmation line for a window change.
fn window_notice(ctx: &Ctx<'_>, account: &Account, window: Option<&ApprovalWindow>) -> String {
    let id = account.id;
    match window {
        Some(w) => format!(
            "Account #{id}: a debit that would take its spend over {} inside any trailing {} s \
             (placements included, refunds never subtracted) needs the same approval.",
            money(ctx, &account.instrument, w.total),
            w.period_secs
        ),
        None => format!(
            "Account #{id}: the rolling window is off; the threshold alone escalates, per \
             transaction."
        ),
    }
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
    let (kind, rest) = lower.split_once(':').ok_or_else(|| {
        CmdError::msg("Bad policy (use threshold:<n>[:<role>] or named:<n>:<obj>,…).")
    })?;
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
            Ok(ApprovalPolicy::Threshold {
                approvals,
                from_role,
            })
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
        _ => Err(CmdError::msg(
            "Bad policy (use threshold:<n>[:<role>] or named:<n>:<obj>,…).",
        )),
    }
}

fn parse_approvals(s: &str) -> Result<u8, CmdError> {
    let n: u8 = s
        .trim()
        .parse()
        .map_err(|_| CmdError::msg("The approval count must be 1-255."))?;
    if n == 0 {
        // A zero-approval rule is satisfied by nothing at all — it escalates and
        // then waves the plan through, which reads as protection and is not.
        return Err(CmdError::msg(
            "An approval count of 0 gates nothing; use `none` to clear the rule.",
        ));
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
    let store = ctx
        .w
        .module_state::<mush_org::OrgStore>()
        .ok_or_else(|| CmdError::msg("No organizations."))?;
    let state = store.state();
    let org = state
        .scope(scope)
        .ok_or_else(|| CmdError::msg("That account's scope no longer exists."))?
        .org;
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
    } else if ctx.switches.has("history") {
        econ_history(ctx)
    } else if ctx.switches.has("approve") {
        econ_decide(ctx, Verdict::Approved)
    } else if ctx.switches.has("deny") {
        econ_decide(ctx, Verdict::Denied)
    } else if ctx.switches.has("approvals") {
        econ_approvals(ctx)
    } else if ctx.switches.has("unfreeze") {
        require_admin(ctx)?;
        econ_unfreeze(ctx)
    } else if ctx.switches.has("alerts") {
        require_admin(ctx)?;
        econ_alerts(ctx)
    } else if ctx.switches.has("ack") {
        require_admin(ctx)?;
        econ_ack(ctx)
    } else if ctx.switches.has("cadence") {
        econ_cadence(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: @econ/pay|mint|reverse|audit|history|approve|deny|approvals|alerts|ack|\
             cadence|unfreeze",
        ))
    }
}

// ---- @econ/cadence (E4b1 deviation 1, shipped with E5b) --------------------

/// `@econ/cadence create|show|list|advance` — the campaign clocks (D-E4-2, owner
/// answer 7).
///
/// E4b1 shipped [`crate::CadenceClock`], its advance arithmetic, the sweep step that
/// moves it and the `cadence:{id}` container doc, and deliberately left the **verbs**
/// to whichever cut first needed a clock a human could name: E5 housing, whose every
/// tenancy is assessed on one. Until now a cadence could only be created by a test
/// rig writing the record directly, which is not a thing a game can do.
///
/// **`create` and `advance` are staff; `list` and `show` are open** (E5b verify fold,
/// A8 — this comment used to say the whole subcommand was staff, which the gate below
/// has never done). A campaign clock is public structure: every tenancy names one,
/// `@tenancy/create cadence=<id>` needs the id, and `@tenancy/show` prints the
/// position, so hiding the list would only mean asking a wizard to read a number the
/// render already shows. Creating one is not the interesting risk either. Advancing a
/// `Manual` one is: `position` is what every schedule on that clock reads as "now",
/// so one advance can make a period of rent due for every tenancy and every
/// obligation pointing at it. That is a staff act by construction, and only for a
/// `Manual` cadence — a real- or IC-sourced clock is the sweep's to move
/// ([`crate::advance_cadence`] returns a `Manual` one unchanged for the same reason).
fn econ_cadence(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @econ/cadence create <name> source=<real:<secs>|ic:<secs>|manual> \
                         [catchup=<n>] | show <id> | list | advance <id> [<n>]";
    let raw = ctx.args.raw().to_string();
    let (rest, opts) = parse_opts(&raw, &["source", "catchup"]);
    let mut words = rest.split_whitespace();
    let verb = words.next().unwrap_or("").to_ascii_lowercase();
    match verb.as_str() {
        "list" => cadence_list(ctx),
        "show" => {
            let id = words
                .next()
                .and_then(parse_cadence_id)
                .ok_or_else(|| CmdError::msg(USAGE))?;
            cadence_show(ctx, id)
        }
        "create" => {
            require_admin(ctx)?;
            refuse_if_frozen(ctx)?;
            let name = words.next().unwrap_or("").trim().to_string();
            let source = opts.get("source").map(String::as_str).unwrap_or("");
            let catchup = opts.get("catchup").and_then(|s| s.parse::<u8>().ok());
            cadence_create(ctx, &name, source, catchup).map_err(CmdError::msg)
        }
        "advance" => {
            require_admin(ctx)?;
            refuse_if_frozen(ctx)?;
            let id = words
                .next()
                .and_then(parse_cadence_id)
                .ok_or_else(|| CmdError::msg(USAGE))?;
            let n = match words.next() {
                None => 1,
                Some(s) => s
                    .parse::<u64>()
                    .ok()
                    .filter(|n| *n > 0 && *n <= 64)
                    .ok_or_else(|| {
                        CmdError::msg("Advance by 1 to 64 periods; a bigger jump is several acts.")
                    })?,
            };
            cadence_advance(ctx, id, n)
        }
        _ => Err(CmdError::msg(USAGE)),
    }
}

fn parse_cadence_id(s: &str) -> Option<CadenceId> {
    s.trim()
        .trim_start_matches('#')
        .parse::<u64>()
        .ok()
        .map(CadenceId)
}

fn cadence_create(
    ctx: &mut Ctx<'_>,
    name: &str,
    source: &str,
    catchup: Option<u8>,
) -> Result<(), String> {
    // The period is part of the source word, because the two are one fact: "every
    // 3600 real seconds", "every 86400 IC seconds", "whenever a human says so".
    let (src, period) = match source.split_once(':') {
        Some(("real", secs)) => (CadenceSource::RealUtc, secs.parse::<i64>().ok()),
        Some(("ic", secs)) => (CadenceSource::WorldIc, secs.parse::<i64>().ok()),
        _ if source.eq_ignore_ascii_case("manual") => (CadenceSource::Manual, Some(1)),
        _ => return Err("source= is real:<secs>, ic:<secs>, or manual.".to_string()),
    };
    let Some(period) = period.filter(|p| *p > 0) else {
        return Err("A cadence period must be a positive number of seconds.".to_string());
    };
    // E5c: one creator. The caps, the duplicate-name rule and the "anchored at now"
    // rule live in `api::create_cadence`, which a ruleset's boot also calls — a
    // campaign clock named in configuration has to exist before the first period is
    // assessed on it, and there is nobody to type this verb at boot.
    let id = api::create_cadence(ctx, name, src, period, catchup).map_err(|e| e.to_string())?;
    info(
        ctx,
        format!(
            "Cadence #{id} created: {name} ({}).",
            source_word(src, period)
        ),
    );
    Ok(())
}

fn source_word(src: CadenceSource, period: i64) -> String {
    match src {
        CadenceSource::RealUtc => format!("every {period}s real"),
        CadenceSource::WorldIc => format!("every {period}s IC"),
        CadenceSource::Manual => "manual".to_string(),
    }
}

fn cadence_show(ctx: &mut Ctx<'_>, id: CadenceId) -> Result<(), CmdError> {
    let (c, declared) = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| {
            let c = s.snapshot().cadence(id).cloned()?;
            // SP1 (D-LBK-7): "declared" is the registration truth — the name is in the
            // live `[economy.cadences]` — not the persisted schema stamp, which only
            // records that some boot adopted it.
            let declared = s.config.declared_cadence(&c.name).is_some();
            Some((c, declared))
        })
        .ok_or_else(|| CmdError::msg("No such cadence."))?;
    info(
        ctx,
        format!(
            "Cadence #{}: {} — {} · {}",
            c.id,
            c.name,
            source_word(c.source, c.period),
            if declared { "declared" } else { "runtime" }
        ),
    );
    info(
        ctx,
        format!(
            "  period {} · catch-up {} · revision {}",
            c.position, c.catch_up_cap, c.revision
        ),
    );
    info(
        ctx,
        format!("  generation {} · schema {}", c.generation, c.schema),
    );
    Ok(())
}

fn cadence_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let rows: Vec<String> = ctx
        .w
        .module_state::<EconomyStore>()
        .map(|s| {
            s.snapshot()
                .cadences()
                .map(|c| {
                    format!(
                        "#{} {} — {} · period {}",
                        c.id,
                        c.name,
                        source_word(c.source, c.period),
                        c.position
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    emit_list(ctx, rows, "No cadence clocks.");
    Ok(())
}

/// `@econ/cadence advance <id> [<n>]` — **`Manual` only** (E4a verify fold, 12).
///
/// Advancing a real- or IC-sourced clock by hand would put its `position` ahead of
/// the source it is derived from, and the very next sweep would compute `elapsed`
/// from `advanced_at` and advance it again: the hand-moved periods would be charged
/// twice, once by the human and once by the clock catching up to itself.
fn cadence_advance(ctx: &mut Ctx<'_>, id: CadenceId, n: u64) -> Result<(), CmdError> {
    let c = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().cadence(id).cloned())
        .ok_or_else(|| CmdError::msg("No such cadence."))?;
    if c.source != CadenceSource::Manual {
        return Err(CmdError::msg(format!(
            "Cadence #{id} advances from its own source ({}); only a manual cadence is stepped by hand.",
            source_word(c.source, c.period)
        )));
    }
    let next = crate::clock::advance_cadence_manual(&c, n);
    let position = next.position;
    api::put_cadence(ctx, next);
    info(ctx, format!("Cadence #{id} is now at period {position}."));
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `@econ/unfreeze` — staff recovery for the **whole-economy** latch (E3 verify
/// finding H). The latch is durable (`meta.frozen`), so this is the only thing that
/// clears it: a boot that froze on drift also heals the drifted projections and
/// persists them, which means no later boot would ever re-derive the freeze.
///
/// It restores exactly the accounts **this latch** froze and no others (E4b1 verify
/// fold, B4a). Boot records them in `meta.boot_frozen`, so an account frozen for its
/// own reason — a destroyed owner, a balance that no longer fits `i64` — stays frozen
/// and stays visible as a thing that needs attention. Before that list existed the
/// only two options were both wrong: unfreeze fifty accounts by hand, or unfreeze
/// them all and lose the record of which ones were really broken. The next sweep
/// then settles normally instead of paging once per stranded obligation.
///
/// Assets and vaults keep their own latches, deliberately: those name which titles
/// and which containers need looking at, and no list can reconstruct that.
fn econ_unfreeze(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_none_or(|s| !s.is_frozen())
    {
        return Err(CmdError::msg("The economy is not frozen."));
    }
    api::set_economy_frozen(ctx, false);
    let restore: Vec<AccountId> = ctx
        .w
        .module_state::<EconomyStore>()
        .map(|s| s.boot_frozen().collect())
        .unwrap_or_default();
    let mut restored = 0usize;
    for id in restore {
        let frozen = ctx.w.module_state::<EconomyStore>().is_some_and(|s| {
            s.snapshot()
                .account(id)
                .is_some_and(|a| a.status == AccountStatus::Frozen)
        });
        if frozen && api::set_account_status(ctx, id, AccountStatus::Active) {
            restored += 1;
        }
    }
    if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
        store.clear_boot_frozen();
    }
    let meta = ctx
        .w
        .module_state::<EconomyStore>()
        .map(EconomyStore::meta_doc);
    if let Some(meta) = meta {
        api::persist_meta(ctx.out, meta);
    }
    info(
        ctx,
        "The whole-economy freeze is cleared. Posting is live again.".to_string(),
    );
    info(
        ctx,
        format!(
            "{restored} account(s) frozen by that latch are active again; any account frozen for \
             its own reason stays frozen (@account/unfreeze <id>)."
        ),
    );
    info(
        ctx,
        "Assets and vaults frozen by the recovery boot keep their own latches: clear them with \
         @asset/unfreeze and @vault/unfreeze."
            .to_string(),
    );
    // E4c verify fold, E4a: canon evidence is edge-triggered, and every record that
    // landed while the freeze was set was dropped on the floor by the attestation
    // observer — deliberately, because a frozen economy cannot act on it. Re-scan
    // what the world announced meanwhile, now that it can.
    crate::attest::rescan_canon_tail(ctx, crate::attest::RescanTrigger::Recovery);
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
    s.trim()
        .trim_start_matches('#')
        .parse::<u64>()
        .ok()
        .map(ApprovalRequestId)
}

/// Whether `actor` may act on a request: would their approval COUNT for at least
/// one of its accounts, under that account's live rule? That is exactly
/// `authorize_approver`, so standing and counting can never drift apart.
///
/// The requester is not excluded here — they may look at and even record a decision
/// on their own request; the engine simply never counts it (four-eyes).
fn may_act_on_request(w: &World, actor: ObjId, request: &crate::record::ApprovalRequest) -> bool {
    let Some(store) = w.module_state::<EconomyStore>() else {
        return false;
    };
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
        return Err(CmdError::msg(format!(
            "Request #{id} is already {}.",
            status_word(request.status)
        )));
    }
    if now >= request.expires_at {
        return Err(CmdError::msg(format!(
            "Request #{id} has expired; the payer must submit a fresh one."
        )));
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
    request.decisions.push(Decision {
        actor: me,
        at: now,
        verdict,
    });
    match verdict {
        Verdict::Approved => {
            if request.approvals.len() >= crate::MAX_APPROVERS_PER_REQUEST {
                return Err(CmdError::msg(format!(
                    "Request #{id} has already recorded the maximum of {} approvals.",
                    crate::MAX_APPROVERS_PER_REQUEST
                )));
            }
            request.approvals.push(Approval {
                approver: me,
                at: now,
            });
        }
        Verdict::Denied => request.status = ApprovalStatus::Cancelled,
    }
    request.revision += 1;
    let requester_is_me = request.requested_by == PrincipalId::Object(me);
    let accounts = request.accounts.clone();
    api::put_approval(ctx, request);
    audit_decision(ctx, verdict, id, &accounts);
    match verdict {
        Verdict::Approved => {
            info(ctx, format!("Approved request #{id}."));
            if requester_is_me {
                info(
                    ctx,
                    "(It is your own request, so it does not count toward the policy.)".to_string(),
                );
            }
        }
        Verdict::Denied => info(ctx, format!("Denied request #{id}; it is cancelled.")),
    }
    Ok(())
}

/// Record an approval decision in the **org** audit ledger (E4c, §6.1 item 4).
///
/// An approval decision is an org-authority act — `may_act_on_request` asks
/// `economy.account.approve` on the account's compartment — whose only record used
/// to be the economy's own `approval:{id}` document, which `@org/audit` cannot see
/// and which GC eventually deletes. The entry names the **request id and the account
/// scope**: not the amount, not the plan, not the payer's balance, because the org
/// ledger is read by org officers rather than by the accounts' owners and the thing
/// they are accountable for is *that they released it*.
///
/// A refused append is logged and swallowed: the decision itself has already been
/// recorded on the request, and losing the audit line must never lose the approval
/// (or return an `Err` that would truncate the whole batch, the approval included).
fn audit_decision(
    ctx: &mut Ctx<'_>,
    verdict: Verdict,
    request: ApprovalRequestId,
    accounts: &[AccountId],
) {
    let scopes: Vec<Option<OrgScopeId>> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        accounts
            .iter()
            .map(|id| store.snapshot().account(*id).and_then(|a| a.scope))
            .collect()
    };
    let org = scopes.iter().flatten().find_map(|s| {
        ctx.w
            .module_state::<mush_org::OrgStore>()
            .and_then(|o| o.state().scope(*s))
            .map(|sc| sc.org)
    });
    let action = mush_org::OrgAction {
        action: SmolStr::new_static(match verdict {
            Verdict::Approved => "economy.approval.approve",
            Verdict::Denied => "economy.approval.deny",
        }),
        org,
        detail: serde_json::json!({
            "request": request.0,
            "accounts": accounts.iter().map(|a| a.0).collect::<Vec<u64>>(),
            "scopes": scopes.iter().map(|s| s.map(|x| x.0)).collect::<Vec<Option<u64>>>(),
        }),
    };
    if let Err(error) = mush_org::audit(ctx, action) {
        tracing::warn!(%error, request = request.0, "economy: an approval decision could not be audited");
    }
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
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
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
            let mine = if r.requested_by == PrincipalId::Object(ctx.actor) {
                " (yours)"
            } else {
                ""
            };
            let already = if r.approvals.iter().any(|a| a.approver == ctx.actor) {
                " [approved]"
            } else {
                ""
            };
            format!(
                "#{} — requested by {}, {} approval(s) so far, account(s) {}{mine}{already}",
                r.id,
                render_principal(ctx, &r.requested_by),
                r.approvals.len(),
                r.accounts
                    .iter()
                    .map(|a| format!("#{a}"))
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        })
        .collect();
    emit_list(ctx, rows, "No approval requests await you.");
    Ok(())
}

/// Parse `<amount>` in an instrument's minor units (integer minor units for E2b —
/// staff type the raw minor value; the display formats the scale).
pub fn parse_amount(s: &str) -> Option<Amount> {
    s.trim().parse::<i64>().ok().map(Amount::from_minor)
}

fn two_accounts_and_amount(
    ctx: &Ctx<'_>,
    usage: &str,
) -> Result<(AccountId, AccountId, Amount), CmdError> {
    let from = parse_account(ctx.args.lhs()).ok_or_else(|| CmdError::msg(usage.to_string()))?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(usage.to_string()))?;
    let mut it = rhs.split_whitespace();
    let to = it
        .next()
        .and_then(parse_account)
        .ok_or_else(|| CmdError::msg(usage.to_string()))?;
    let amount = it
        .next()
        .and_then(parse_amount)
        .ok_or_else(|| CmdError::msg(usage.to_string()))?;
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
    const USAGE: &str =
        "Usage: @econ/pay <from>=<to> <amount> [asset=<obj>] [from_vault=<id>|to_vault=<id>]";
    let from = parse_account(ctx.args.lhs()).ok_or_else(|| CmdError::msg(USAGE))?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let (rest, opts) = parse_opts(rhs, &["asset", "from_vault", "to_vault"]);
    let mut it = rest.split_whitespace();
    let to = it
        .next()
        .and_then(parse_account)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let amount = it
        .next()
        .and_then(parse_amount)
        .ok_or_else(|| CmdError::msg(USAGE))?;
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
        let source = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().account(from).cloned());
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
            let store = ctx
                .w
                .module_state::<EconomyStore>()
                .ok_or_else(|| CmdError::msg("No economy."))?;
            let snap = store.snapshot();
            let payer = snap.account(from).map(|a| a.owner.clone());
            let dest = snap
                .account(to)
                .ok_or_else(|| CmdError::msg("No such account."))?;
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
        asset_ops.push(AssetOp::TransferTitle {
            object,
            to: recipient,
            to_scope,
        });
        match (opts.get("to_vault"), opts.get("from_vault")) {
            (Some(_), Some(_)) => {
                return Err(CmdError::msg(
                    "Use at most one of to_vault= and from_vault=.",
                ));
            }
            (Some(v), None) => {
                let vault = parse_vault_id(v).ok_or_else(|| CmdError::msg("No such vault."))?;
                custody_ops.push(CustodyOp::Deposit { object, vault });
            }
            (None, Some(v)) => {
                let vault = parse_vault_id(v).ok_or_else(|| CmdError::msg("No such vault."))?;
                custody_ops.push(CustodyOp::Withdraw {
                    object,
                    vault,
                    to: ctx.actor,
                });
            }
            (None, None) => {}
        }
    } else if opts.contains_key("to_vault") || opts.contains_key("from_vault") {
        return Err(CmdError::msg("A vault leg needs asset=<obj>."));
    }

    let neg = amount
        .checked_neg()
        .ok_or_else(|| CmdError::msg("Amount out of range."))?;
    let mut plan = EconomyPlan::new(
        cmd_key(ctx),
        PrincipalId::Object(ctx.actor),
        EconomicCause::Command {
            serial: ctx.w.next_correlation_id(),
        },
        vec![
            RequestedLine {
                account: from,
                delta: neg,
                memo: SmolStr::default(),
            },
            RequestedLine {
                account: to,
                delta: amount,
                memo: SmolStr::default(),
            },
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
            info(
                ctx,
                format!("Approval required: request #{id}. Nothing has moved."),
            );
            info(
                ctx,
                "Once it is approved, run the same command again to post it.".to_string(),
            );
            Ok(())
        }
        Err(e) => Err(refuse(
            ctx,
            NamedResource::Account(from),
            &[NamedResource::Account(to)],
            &e,
        )),
    }
}

fn econ_mint(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (source, to, amount) =
        two_accounts_and_amount(ctx, "Usage: @econ/mint <sysacct>=<to> <amount>")?;
    let neg = amount
        .checked_neg()
        .ok_or_else(|| CmdError::msg("Amount out of range."))?;
    let plan = EconomyPlan::new(
        cmd_key(ctx),
        PrincipalId::Object(ctx.actor),
        EconomicCause::Staff {
            reason: "mint".into(),
        },
        vec![
            RequestedLine {
                account: source,
                delta: neg,
                memo: SmolStr::new_static("mint"),
            },
            RequestedLine {
                account: to,
                delta: amount,
                memo: SmolStr::new_static("grant"),
            },
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
    let cause = EconomicCause::Staff {
        reason: "reversal".into(),
    };
    let outcome = api::reverse(ctx, txid, PrincipalId::Object(ctx.actor), cause.clone());
    // J2: a target whose body left the RAM tail is read from the journal off the
    // world loop and reversed when the chunk comes back (`EconomyArchiveRead`).
    if let Err(crate::plan::EconomyError::TransactionNotInMemory(target)) = &outcome {
        let target = *target;
        // Doc 13 W2 (W-D6): by sequence while the index has the id, by the chunk
        // map once the window evicted it.
        let chunk = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.chunk_of_tx(target))
            .ok_or_else(|| CmdError::msg(format!("no such transaction: {target}")))?;
        let kind = PendingEconomyReadKind::Reverse { target, cause };
        return request_cold_read(ctx, chunk, kind, |chunk| {
            format!(
                "Transaction #{target} is older than the in-memory tail; reading it from the journal (chunk {chunk}) …"
            )
        });
    }
    finish(ctx, outcome, "Reversed")
}

/// Queue one cold journal read for the chunk holding `seq` (J2; the H1 seam):
/// one read in flight per actor, a stale one superseded. `say` renders the
/// line the actor sees while the read is out.
fn request_cold_read(
    ctx: &mut Ctx<'_>,
    chunk: u64,
    kind: PendingEconomyReadKind,
    say: impl FnOnce(u64) -> String,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let now = ctx.w.now();
    {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("The economy is not loaded."))?;
        if let Some(p) = store.pending_reads.get(&actor)
            && now.saturating_sub(p.requested_at) < mush_core::STORE_READ_STALE_SECS
        {
            return Err(CmdError::msg(
                "A journal read is already in flight for you; wait for it to answer.",
            ));
        }
    }
    let correlation = ctx
        .read_store(vec![mush_core::chunk_key(JOURNAL_LOG, chunk)])
        .map_err(|error| {
            CmdError::msg(format!("That journal page cannot be requested: {error}."))
        })?;
    if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
        store.pending_reads.insert(
            actor,
            PendingEconomyRead {
                correlation,
                requested_at: now,
                kind,
            },
        );
    }
    info(ctx, say(chunk));
    Ok(())
}

/// Queue the next page of older history for `@econ/history` (J2): the
/// `HISTORY_PAGE_CHUNKS` chunks ending at `hi`.
fn request_history_page(
    ctx: &mut Ctx<'_>,
    hi: u64,
    account: AccountId,
    wanted: usize,
    staff: bool,
    pages_left: u8,
    below_seq: u64,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let now = ctx.w.now();
    {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("The economy is not loaded."))?;
        if let Some(p) = store.pending_reads.get(&actor)
            && now.saturating_sub(p.requested_at) < mush_core::STORE_READ_STALE_SECS
        {
            return Err(CmdError::msg(
                "A journal read is already in flight for you; wait for it to answer.",
            ));
        }
    }
    let lo = hi.saturating_sub(HISTORY_PAGE_CHUNKS - 1);
    let keys: Vec<SmolStr> = (lo..=hi)
        .map(|n| mush_core::chunk_key(JOURNAL_LOG, n))
        .collect();
    let correlation = ctx
        .read_store(keys)
        .map_err(|error| CmdError::msg(format!("Older history cannot be requested: {error}.")))?;
    if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
        store.pending_reads.insert(
            actor,
            PendingEconomyRead {
                correlation,
                requested_at: now,
                kind: PendingEconomyReadKind::History {
                    account,
                    wanted,
                    staff,
                    next_hi: hi,
                    pages_left,
                    below_seq,
                },
            },
        );
    }
    info(
        ctx,
        format!("Reading older history from the journal (chunks {lo}–{hi}) …"),
    );
    Ok(())
}

/// How many chunks one older-history page reads (J2): about 110 entries at the
/// 16 KiB chunk target, H1's archive page.
const HISTORY_PAGE_CHUNKS: u64 = mush_core::DEFAULT_ARCHIVE_PAGE_CHUNKS;
/// How many pages one `@econ/history` may walk past the RAM tail before it
/// stops with a line (J2): bounded, so an account with no recent activity
/// cannot make one command read the whole journal.
const HISTORY_MAX_PAGES: u8 = 8;

/// The economy's [`mush_core::StoreReadCompletion`] (J2): finish the command
/// that reached past the RAM tail, through the same renderers the warm path
/// uses. Standing is re-checked here (treaty ruling 3): the actor must still
/// exist, and for a history page still be entitled to view the account.
pub(crate) struct EconomyArchiveRead;

impl mush_core::StoreReadCompletion for EconomyArchiveRead {
    fn on_read(&self, ctx: &mut Ctx<'_>, correlation: u64, result: mush_core::StoreReadResult) {
        let actor = ctx.actor;
        // Doc 13: a keyed `idem:` read (the gate's slow lookup) is matched by
        // correlation, never by actor, and its answer warms the cold cache.
        let keyed = ctx.w.module_state::<EconomyStore>().and_then(|store| {
            store
                .pending_key_reads
                .iter()
                .find(|(_, p)| p.correlation == correlation)
                .map(|(k, _)| k.clone())
        });
        if let Some(key) = keyed {
            crate::keys::complete_key_read(ctx, &key, result);
            return;
        }
        let pending = {
            let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else {
                return;
            };
            match store.pending_reads.get(&actor) {
                Some(p) if p.correlation == correlation => store.pending_reads.remove(&actor),
                Some(p) => {
                    tracing::warn!(
                        %actor,
                        correlation,
                        pending = p.correlation,
                        "economy: stale journal completion dropped"
                    );
                    return;
                }
                None => {
                    tracing::warn!(
                        %actor,
                        correlation,
                        "economy: journal completion with no pending read dropped"
                    );
                    return;
                }
            }
        };
        let Some(pending) = pending else {
            return;
        };
        if ctx.w.get(actor).is_none() {
            return;
        }
        let (docs, absent) = match result {
            mush_core::StoreReadResult::Docs { docs, absent } => (docs, absent),
            mush_core::StoreReadResult::Failed(detail) => {
                info(ctx, format!("The journal read failed: {detail}."));
                return;
            }
            mush_core::StoreReadResult::Oversized { bytes } => {
                info(
                    ctx,
                    format!("The journal page was too large to read ({bytes} bytes)."),
                );
                return;
            }
        };
        // A chunk below the RAM tail is sealed history the store holds; one that
        // is absent is history the store no longer has (or never did), which no
        // command may paper over.
        if !absent.is_empty() {
            info(
                ctx,
                format!(
                    "That part of the journal is not in the store ({} chunk(s) absent).",
                    absent.len()
                ),
            );
            return;
        }
        let entries = match mush_core::read_cold_range(&docs, JOURNAL_LOG) {
            Ok(entries) => entries,
            Err(error) => {
                info(ctx, format!("The journal page did not validate: {error}."));
                return;
            }
        };
        let mut transactions: Vec<crate::record::EconomicTransaction> =
            Vec::with_capacity(entries.len());
        for entry in entries {
            match serde_json::from_value::<crate::record::EconomicTransaction>(entry.value) {
                Ok(tx) => transactions.push(tx),
                Err(error) => {
                    info(
                        ctx,
                        format!(
                            "The journal page holds an unreadable entry (seq {}): {error}.",
                            entry.seq
                        ),
                    );
                    return;
                }
            }
        }
        match pending.kind {
            PendingEconomyReadKind::Reverse { target, cause } => {
                let Some(original) = transactions.into_iter().find(|t| t.id == target) else {
                    info(
                        ctx,
                        format!("Transaction #{target} was not in the chunk the journal named."),
                    );
                    return;
                };
                if !is_admin(ctx.w, actor) {
                    info(ctx, "Permission denied.".to_string());
                    return;
                }
                let outcome =
                    api::reverse_original(ctx, &original, PrincipalId::Object(actor), cause);
                if let Err(e) = finish(ctx, outcome, "Reversed") {
                    info(ctx, e.to_string());
                }
            }
            PendingEconomyReadKind::History {
                account,
                wanted,
                staff,
                next_hi,
                pages_left,
                below_seq,
            } => {
                if !may_view_account(ctx.w, actor, account) {
                    info(ctx, "No such account.".to_string());
                    return;
                }
                let staff = staff && is_admin(ctx.w, actor);
                let mut touching: Vec<crate::record::EconomicTransaction> = transactions
                    .into_iter()
                    .filter(|t| t.journal_seq < below_seq && history_touches(t, account, staff))
                    .collect();
                touching.reverse();
                let found = touching.len().min(wanted);
                touching.truncate(found);
                for row in history_rows(ctx, &touching, staff) {
                    info(ctx, row);
                }
                let remaining = wanted - found;
                let more_below = next_hi >= HISTORY_PAGE_CHUNKS;
                if remaining == 0 || !more_below {
                    if remaining > 0 {
                        info(ctx, "That is the beginning of the journal.".to_string());
                    }
                    return;
                }
                if pages_left == 0 {
                    info(
                        ctx,
                        format!(
                            "Stopped after {HISTORY_MAX_PAGES} pages of older history; ask for fewer entries or a later account."
                        ),
                    );
                    return;
                }
                let next_hi = next_hi - HISTORY_PAGE_CHUNKS;
                if let Err(e) = request_history_page(
                    ctx,
                    next_hi,
                    account,
                    remaining,
                    staff,
                    pages_left - 1,
                    below_seq,
                ) {
                    info(ctx, e.to_string());
                }
            }
        }
    }
}

fn econ_audit(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lines: Vec<String> = {
        let store = ctx.w.module_state::<EconomyStore>().expect("economy store");
        let all: Vec<&crate::record::EconomicTransaction> = store.transactions().collect();
        let mut v: Vec<String> = all
            .iter()
            .rev()
            .take(20)
            .map(|t| {
                format!(
                    "  tx #{} seq {} ({} line(s)){}",
                    t.id,
                    t.journal_seq,
                    t.lines.len(),
                    t.reverses
                        .map_or_else(String::new, |r| format!(" reverses #{r}"))
                )
            })
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

// ---- @econ/history — the disclosure filter (E4c, §6.1 item 5) --------------

/// The most entries `@econ/history` will ever render, whatever `n` says.
const HISTORY_CAP: usize = 20;

/// Whether this viewer is a **party** to one journal entry — the class that sees its
/// free-text description.
///
/// A party is someone the entry's money is *about*: whoever posted it, or whoever
/// acts for the owner of an account it moved. `acts_for` is the predicate (not
/// identity), for the reason the E4b1 fold gave for holds: an organization's
/// spender is a party to its org's payment, and a Thing's controller to the
/// counter's. It is deliberately **narrower than viewability**: an auditor holding
/// `economy.account.view` may read what moved on the account they audit without
/// being told, in free text, what a staffer wrote about why.
fn entry_party(ctx: &Ctx<'_>, tx: &crate::record::EconomicTransaction) -> bool {
    if is_admin(ctx.w, ctx.actor) {
        return true;
    }
    let me = PrincipalId::Object(ctx.actor);
    let auth = api::WorldAuth { w: ctx.w };
    if auth.acts_for(&me, &tx.actor) {
        return true;
    }
    let Some(store) = ctx.w.module_state::<EconomyStore>() else {
        return false;
    };
    let snap = store.snapshot();
    tx.lines
        .iter()
        .filter_map(|l| snap.account(l.account))
        .any(|a| auth.acts_for(&me, &a.owner))
}

/// Whether this viewer may read the entry's **description**, per its recorded
/// [`crate::EconomicVisibility`] (E4c: visibility becomes enforced for reads).
///
/// `Public` tells anyone; `Parties` tells the parties; an `Audience` tells the
/// parties **and** the named audience, and is treated exactly like `Parties` for
/// everyone outside it — doc 10 T6's audience is a class that gets *notice*, and a
/// description is the nearest thing an entry has to terms. `Staff` never reaches a
/// non-staff reader at all, so it does not appear here.
fn entry_descriptive(ctx: &Ctx<'_>, tx: &crate::record::EconomicTransaction) -> bool {
    match &tx.visibility {
        crate::record::EconomicVisibility::Public => true,
        crate::record::EconomicVisibility::Audience { members, .. } => {
            let me = PrincipalId::Object(ctx.actor);
            let auth = api::WorldAuth { w: ctx.w };
            members.iter().any(|p| auth.acts_for(&me, p)) || entry_party(ctx, tx)
        }
        _ => entry_party(ctx, tx),
    }
}

/// What a hold effect did, as one word.
fn hold_word(kind: HoldEffectKind) -> &'static str {
    match kind {
        HoldEffectKind::Placed => "reserved",
        HoldEffectKind::Captured => "captured",
        HoldEffectKind::Released => "released",
        HoldEffectKind::Expired => "expired",
    }
}

/// `@econ/history <account> [n]` — the read side of [`crate::EconomicVisibility`].
///
/// The rules, all of them, because this is the command the "evidence leaks" verify
/// axis is aimed at:
///
/// 1. **Standing first.** An account the actor may not view and an account that does
///    not exist render the identical `No such account.`, exactly as `@hold/list` and
///    `@account/show` already do. Nothing below can run for an id that failed here,
///    so no message length, count or timing distinguishes the two.
/// 2. **The viewer's own side in full**, because it is theirs: every line on an
///    account they may view carries its real id and its real signed amount.
/// 3. **A counterparty is named only if they may view it too.** Otherwise it renders
///    as `another account` **with no amount** — a five-way split would otherwise tell
///    a runner what every other runner was paid, which is exactly the disclosure a
///    `Parties` contract did not consent to. (In a two-line transfer the mirror
///    amount is arithmetic, not disclosure; the id is what is withheld.)
/// 4. **`Staff` entries are omitted entirely**, not shown as hidden. A "3 entries you
///    may not see" line is an oracle for how much staff activity an account attracts.
/// 5. **Descriptions only to parties** ([`entry_descriptive`]).
/// 6. **Newest first, capped at [`HISTORY_CAP`]**, whatever `n` asks for.
///
/// Staff read it as the audit view: every entry, every id, every description.
fn econ_history(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const NO_SUCH: &str = "No such account.";
    const USAGE: &str = "Usage: @econ/history <account> [n]   (n is 1-20)";
    let raw = ctx.args.raw().to_string();
    let mut words = raw.split_whitespace();
    let id = words
        .next()
        .and_then(parse_account)
        .ok_or_else(|| CmdError::msg(NO_SUCH))?;
    // D8 (E4c verify fold): `n` is validated, never clamped. Clamping split the bad
    // inputs into two observable classes — `0` silently became 1 and `banana`
    // silently became 20 — so the reply told you which kind of wrong you were.
    let n = match words.next() {
        None => HISTORY_CAP,
        Some(word) => match word.parse::<usize>() {
            Ok(n) if (1..=HISTORY_CAP).contains(&n) => n,
            _ => return Err(CmdError::msg(USAGE)),
        },
    };
    if words.next().is_some() {
        return Err(CmdError::msg(USAGE));
    }
    // Rule 1: standing, then existence — one message for both.
    if !may_view_account(ctx.w, ctx.actor, id) {
        return Err(CmdError::msg(NO_SUCH));
    }
    let staff = is_admin(ctx.w, ctx.actor);
    let (entries, older) = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg(NO_SUCH))?;
        if store.snapshot().account(id).is_none() {
            return Err(CmdError::msg(NO_SUCH));
        }
        let mut touching: Vec<crate::record::EconomicTransaction> = store
            .transactions()
            .filter(|t| history_touches(t, id, staff))
            .cloned()
            .collect();
        // Rule 6: newest first, then capped. (`transactions()` is id order, and ids
        // are monotonic, so reversing is chronological.)
        touching.reverse();
        touching.truncate(n);
        // J2: the RAM tail is the newest slice of the journal. When it did not
        // fill the request and the journal goes further back, the rest is read
        // from the sealed chunks below the tail, off the world loop.
        let older = match store.tail_floor_seq() {
            Some(floor) if touching.len() < n && floor > 0 => {
                store.chunk_of_seq(floor - 1).map(|hi| (hi, floor))
            }
            _ => None,
        };
        (touching, older)
    };
    let rows = history_rows(ctx, &entries, staff);
    // Rule 6: newest first is the order they were collected in. `emit_list` does
    // **not** sort (it caps and counts), and this render is deliberately uncapped
    // and unsorted — the rows go out exactly as collected (look-back fold, E4c-C109).
    if rows.is_empty() && older.is_none() {
        info(ctx, format!("Nothing recorded against account #{id}."));
    }
    for r in rows {
        info(ctx, r);
    }
    if let Some((hi, below_seq)) = older {
        let wanted = n - entries.len();
        request_history_page(ctx, hi, id, wanted, staff, HISTORY_MAX_PAGES, below_seq)?;
    }
    Ok(())
}

/// Whether `t` belongs in `account`'s history as `staff` (or not) may see it
/// (J2: shared by the warm path and the cold pages).
///
/// "Touching" includes an **effect-only** entry: an escrow placement moves
/// `held_amount` and no line at all, and it is emphatically something that
/// happened to this account. Rule 4, **relaxed for an effect-only entry** (D3):
/// a reservation on an account is a fact its viewers may see, and an escrow
/// placement produces no journal line at all — so omitting the whole entry hid
/// the one thing the payer most needs to see. The entry is shown; what it was
/// FOR is not (no description, and `render_effect` names no contract).
fn history_touches(
    t: &crate::record::EconomicTransaction,
    account: AccountId,
    staff: bool,
) -> bool {
    let touches = t.lines.iter().any(|l| l.account == account)
        || t.hold_effects.iter().any(|e| e.hold.account == account);
    touches
        && (staff
            || !matches!(t.visibility, crate::record::EconomicVisibility::Staff)
            || t.lines.is_empty())
}

/// Render history rows through the one renderer the warm path uses (J2:
/// shared with the cold pages), newest first as given.
fn history_rows(
    ctx: &Ctx<'_>,
    entries: &[crate::record::EconomicTransaction],
    staff: bool,
) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    for t in entries {
        let mut parts: Vec<String> = Vec::new();
        let mut masked = false;
        for l in &t.lines {
            // Rules 2 and 3, through the one renderer (E6b, B1).
            if let Some(line) = render_account_entry(ctx, l.account, &l.instrument, l.delta) {
                parts.push(line);
            } else if !masked {
                // D4 (E4c verify fold): every masked line on one entry COLLAPSES
                // into a single token. One `another account` per hidden line
                // published the arity of the split — a runner could count the other
                // runners — and with the viewer's own side and the total in hand, a
                // two-hidden-line entry was solvable by subtraction. One token says
                // the same true thing and counts nothing. (The finding asked for
                // *consecutive* runs; collapsing them all is strictly stronger, and
                // a run whose masked lines straddle the viewer's own would otherwise
                // still publish "there are others on both sides".)
                masked = true;
                parts.push(MASKED_ACCOUNTS.to_string());
            }
        }
        // E6b independent verify fold (D7): the **arity** of the hidden half is not
        // published either. The line half collapses every masked account into one
        // `MASKED_ACCOUNTS` token precisely so a runner cannot count the other
        // runners, and the hold half sat below it emitting one "a reservation … on
        // another account" per hidden hold — the same count, by a different route.
        // One token, the same way.
        let mut masked_holds = false;
        for e in &t.hold_effects {
            // A reservation is read through its own view standing (`may_view_hold`)
            // AND the account's: the id of somebody else's escrow is exactly the sort
            // of handle a prober wants.
            if may_view_hold(ctx.w, ctx.actor, e.hold.id)
                && may_view_account(ctx.w, ctx.actor, e.hold.account)
            {
                parts.push(format!(
                    "hold #{} {} {} on account #{}",
                    e.hold.id,
                    hold_word(e.kind),
                    money_on(ctx, e.hold.account, e.hold.amount),
                    e.hold.account
                ));
            } else if !masked_holds {
                masked_holds = true;
                parts.push(format!("reservations on {MASKED_ACCOUNTS}"));
            }
        }
        rows.push(format!(
            "tx #{} at {}: {}",
            t.id,
            t.posted_at,
            parts.join(", ")
        ));
        // Rule 5 — and never for a `Staff`-visibility entry a non-staff viewer only
        // sees because it is effect-only (D3): the reservation is the disclosure, the
        // free text about why is not.
        let staff_only = matches!(t.visibility, crate::record::EconomicVisibility::Staff);
        if !t.description.is_empty() && (staff || !staff_only) && entry_descriptive(ctx, t) {
            rows.push(format!("  \u{201c}{}\u{201d}", t.description));
        }
    }
    rows
}

fn finish(
    ctx: &mut Ctx<'_>,
    outcome: Result<PostOutcome, crate::plan::EconomyError>,
    verb: &str,
) -> Result<(), CmdError> {
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
    } else if ctx.switches.has("stolen") {
        asset_stolen(ctx)
    } else if ctx.switches.has("catalog") {
        asset_catalog(ctx)
    } else if ctx.switches.has("abandon") {
        asset_abandon(ctx)
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
            "Usage: @asset/title|transfer|assign|unassign|maintenance|lien|release|catalog|abandon|writeoff|unfreeze|show|list",
        ))
    }
}

/// `@asset/catalog <obj>=<ruleset>:<code>|none` (staff) — E6b independent verify
/// fold, M7.
///
/// `CatalogRef` was writable only by `AssetOp::Title`, and `Title` refuses an object
/// that already has a live record — so **every title minted before E6b could never be
/// catalogued**, and every catalogue-keyed rule was unreachable for it (the fence
/// refuses an object with no `catalog`, which is the whole of the shipped
/// consequence). The refusals go through the existence-hiding renderer like every
/// other `@asset` verb.
fn asset_catalog(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @asset/catalog <obj>=<ruleset>:<code>|none";
    let object = asset_target(ctx, ctx.args.lhs())?;
    if !may_view_asset(ctx.w, ctx.actor, object) {
        return Err(refuse_named(
            ctx,
            NamedResource::Asset(object),
            [],
            "No such asset.".to_string(),
        ));
    }
    let rhs = ctx
        .args
        .rhs()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if rhs.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    let catalog = if rhs.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(parse_catalog(&rhs).ok_or_else(|| CmdError::msg(USAGE))?)
    };
    let plan = ops_plan(
        ctx,
        vec![AssetOp::SetCatalog { object, catalog }],
        Vec::new(),
    );
    let outcome = api::post(ctx, &plan);
    finish_asset(ctx, object, outcome, "Catalogue reference set")
}

/// `<principal> [scope=<id>]` — the owner half shared by `title` and `transfer`.
fn owner_and_scope(
    ctx: &Ctx<'_>,
    rhs: &str,
) -> Result<(PrincipalId, Option<OrgScopeId>), CmdError> {
    let (rest, opts) = parse_opts(rhs, &["scope"]);
    let owner =
        parse_principal(ctx, rest.trim()).ok_or_else(|| CmdError::msg("No such principal."))?;
    let scope = match opts.get("scope") {
        Some(s) => Some(parse_scope(s).ok_or_else(|| CmdError::msg("Bad scope id."))?),
        None => None,
    };
    Ok((owner, scope))
}

fn asset_title(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str =
        "Usage: @asset/title <obj>=<principal> [scope=<id>] [catalog=<ruleset>:<code>]";
    let object = asset_target(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    // E6b: the opaque `CatalogRef` was **set nowhere** before this — the type existed
    // and no producer filled it — which made every catalog-keyed rule (E6b's fence
    // baseline, E6c's whole market) unreachable. It is one option on the existing
    // spec, not a verb: the economy still never interprets it.
    let (rhs, opts) = parse_opts(rhs, &["catalog"]);
    let catalog = match opts.get("catalog") {
        Some(s) => Some(parse_catalog(s).ok_or_else(|| CmdError::msg(USAGE))?),
        None => None,
    };
    let (owner, scope) = owner_and_scope(ctx, &rhs)?;
    let plan = ops_plan(
        ctx,
        vec![AssetOp::Title {
            object,
            owner,
            scope,
            catalog,
        }],
        Vec::new(),
    );
    let outcome = api::post(ctx, &plan);
    finish_asset(ctx, object, outcome, "Title recorded")
}

fn asset_transfer(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let object = asset_target(ctx, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg("Usage: @asset/transfer <obj>=<principal> [scope=<id>]"))?;
    let (to, to_scope) = owner_and_scope(ctx, rhs)?;
    let plan = ops_plan(
        ctx,
        vec![AssetOp::TransferTitle {
            object,
            to,
            to_scope,
        }],
        Vec::new(),
    );
    let outcome = api::post(ctx, &plan);
    finish_asset(ctx, object, outcome, "Title transferred")
}

fn asset_assign(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let object = asset_target(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| {
        CmdError::msg("Usage: @asset/assign <obj>=<principal> [purpose] [until=<unix>]")
    })?;
    let (rest, opts) = parse_opts(rhs, &["until"]);
    let mut words = rest.split_whitespace();
    let who = words
        .next()
        .ok_or_else(|| CmdError::msg("Assign to whom?"))?;
    let to = parse_principal(ctx, who).ok_or_else(|| CmdError::msg("No such principal."))?;
    let purpose: String = words.collect::<Vec<_>>().join(" ");
    let valid_until = match opts.get("until") {
        Some(s) => Some(
            s.parse::<i64>()
                .map_err(|_| CmdError::msg("Bad until= time."))?,
        ),
        None => None,
    };
    let plan = ops_plan(
        ctx,
        vec![AssetOp::Assign {
            object,
            to,
            purpose: SmolStr::new(purpose),
            valid_until,
        }],
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
    let plan = ops_plan(
        ctx,
        vec![AssetOp::SetMaintenance { object, account }],
        Vec::new(),
    );
    let outcome = api::post(ctx, &plan);
    // The payer account is a subject too: invariant 14's refusal ("scope and owner
    // do not agree") is an answer ABOUT that account, and naming a wallet the actor
    // may not view is how a probe would find one.
    let also: Vec<NamedResource> = account.map(NamedResource::Account).into_iter().collect();
    finish_ops(
        ctx,
        NamedResource::Asset(object),
        &also,
        outcome,
        "Maintenance payer set",
    )
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
    let amount = words
        .next()
        .and_then(parse_amount)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let instrument = words
        .next()
        .and_then(InstrumentId::new)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let plan = ops_plan(
        ctx,
        vec![AssetOp::Encumber {
            object,
            encumbrance: Encumbrance::Lien {
                creditor,
                amount,
                instrument,
            },
        }],
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
    let (encumbrance, hidden_claim) = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        let asset = store.snapshot().asset(object);
        let encumbrance = asset.and_then(|a| a.encumbrances.get(n - 1).cloned());
        // The claim a **hidden** theft report placed, reconstructed exactly as
        // `AssetOp::ReportStolen` writes it.
        let hidden_claim = asset.and_then(|a| a.stolen.as_ref()).map(|r| {
            (
                Encumbrance::Disputed {
                    claimant: r.by.clone(),
                    reason: r.reason.clone(),
                },
                r.clone(),
            )
        });
        (encumbrance, hidden_claim)
    };
    // E6b independent verify fold (D2): **the index is not an oracle.** The resolve
    // ran with no view gate at all, so an out-of-range index answered "No such
    // encumbrance." and an in-range one answered "unauthorized" — and an assignee
    // holds `may_view_asset`, so a covert theft report was readable off which of the
    // two sentences came back. A claim the viewer may not be told about answers
    // exactly as an index past the end does, which is the rule D2's own count fix
    // already applies one line above the report in `@asset/show`.
    if let (Some(e), Some((claim, report))) = (encumbrance.as_ref(), hidden_claim.as_ref())
        && e == claim
        && !may_view_stolen_report(ctx, object, report)
    {
        return Err(asset_refusal(ctx, object, "No such encumbrance."));
    }
    let Some(encumbrance) = encumbrance else {
        return Err(asset_refusal(ctx, object, "No such encumbrance."));
    };
    let plan = ops_plan(
        ctx,
        vec![AssetOp::Release {
            object,
            encumbrance,
        }],
        Vec::new(),
    );
    let outcome = api::post(ctx, &plan);
    finish_asset(ctx, object, outcome, "Claim released")
}

/// `@asset/abandon <obj>` (E7 cut 3; doc 10 T3, §15).
///
/// A **switch on the existing `@asset` spec**, so `CommandSpec::name` is
/// unchanged, the rule-1 allowlist needs no stanza for it and `PINNED_BASELINE`
/// is untouched. It rides the `EconomyCarveOut` lane the whole `@asset` surface
/// already declares.
///
/// Title is **retained** as the last owner (provenance survives, T3) and
/// **custody is unchanged**; the asset then authorizes nobody and takes no
/// asset op but its title. A re-run is a HIT: the idempotency key is
/// `asset.abandon/{obj}` and a second post resolves through the stored record.
fn asset_abandon(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let object = asset_target(ctx, ctx.args.lhs())?;
    let plan = ops_plan(ctx, vec![AssetOp::Abandon { object }], Vec::new());
    let outcome = api::post(ctx, &plan);
    finish_asset(ctx, object, outcome, "Abandoned")
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

// ---- E6b: B2 — `@asset/stolen` --------------------------------------------

/// `@asset/stolen <obj>=<public|restricted|covert> [reason]` (E6b, D-E6-18).
///
/// A **switch on the existing `@asset` spec**, so `CommandSpec::name` is unchanged
/// and the rule-1 allowlist needs no stanza for it. Every refusal — an object with
/// no title record, one the actor may not view, one they may not file on — goes
/// through the existence-hiding renderer, so "no such asset" and "not yours to
/// claim" are the same sentence.
fn asset_stolen(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @asset/stolen <obj>=<public|restricted|covert> [reason]";
    let object = asset_target(ctx, ctx.args.lhs())?;
    // Existence hiding first: an actor who may not view the asset learns nothing
    // from the shape of the refusal — and it goes through the renderer, so C16's
    // per-actor tally sees a probe walking an id space.
    if !may_view_asset(ctx.w, ctx.actor, object) {
        return Err(refuse_named(
            ctx,
            NamedResource::Asset(object),
            [],
            "No such asset.".to_string(),
        ));
    }
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let mut words = rhs.split_whitespace();
    let visibility = words
        .next()
        .and_then(crate::record::StolenVisibility::parse)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let reason: String = words.collect::<Vec<_>>().join(" ");
    // The claimant is **derived**, never input (E6a M6's rule): the asset's own
    // title owner. An insurer files on the owner's behalf; a covert report is still
    // a claim about that title.
    let claimant = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No such asset."))?;
        store
            .snapshot()
            .asset(object)
            .ok_or_else(|| CmdError::msg("No such asset."))?
            .title_owner
            .clone()
    };
    let plan = ops_plan(
        ctx,
        vec![AssetOp::ReportStolen {
            object,
            claimant,
            visibility,
            reason,
        }],
        Vec::new(),
    );
    let outcome = api::post(ctx, &plan);
    finish_asset(ctx, object, outcome, "Reported stolen")
}

/// Whether `actor` may be told about `asset`'s standing theft report (answer 14).
///
/// `Public` is anyone who can see the asset at all; `Restricted` needs view
/// standing over the asset; `Covert` is the reporter and staff only — invisible to
/// the thief even while they hold the object, which is the whole point.
#[must_use]
pub fn may_view_stolen_report(
    ctx: &Ctx<'_>,
    object: ObjId,
    report: &crate::record::StolenReport,
) -> bool {
    match report.visibility {
        crate::record::StolenVisibility::Public => true,
        crate::record::StolenVisibility::Restricted => may_view_asset(ctx.w, ctx.actor, object),
        // E6b verify fold (D7): **and an authorized insurer.** A holder of
        // `asset.insure` on the asset's compartment is one of exactly three who may
        // *file* a report (D-E6-18), and the engine's duplicate refusal
        // (`AlreadyClaimed`) told them a covert one stood whether or not this said
        // they could see it — so the disclosure happened anyway, as an error message
        // rather than as a rendered line. Covert means invisible to the **thief**,
        // not to the insurer whose claim it is.
        crate::record::StolenVisibility::Covert => {
            api::is_staff(ctx.w, ctx.actor)
                || api::WorldAuth { w: ctx.w }.acts_for(&PrincipalId::Object(ctx.actor), &report.by)
                || insures(ctx, object)
        }
    }
}

/// Whether the actor holds `asset.insure` on this asset's own live compartment
/// (E6b, D-E6-18) — the third of the three who may file a theft report on it.
#[must_use]
fn insures(ctx: &Ctx<'_>, object: ObjId) -> bool {
    let Some(store) = ctx.w.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(asset) = store.snapshot().asset(object) else {
        return false;
    };
    let Some(scope) = asset.scope else {
        return false;
    };
    api::scope_coherent(ctx.w, Some(scope), &asset.title_owner)
        && api::holds_capability(ctx.w, ctx.actor, "asset.insure", scope)
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
            let store = ctx
                .w
                .module_state::<EconomyStore>()
                .ok_or_else(|| CmdError::msg("No such asset."))?;
            store
                .snapshot()
                .asset(object)
                .ok_or_else(|| CmdError::msg("No such asset."))?
                .clone()
        };
        let mut v = vec![format!(
            "asset #{} — title {} [{:?}]",
            a.object.0,
            render_principal(ctx, &a.title_owner),
            a.lifecycle
        )];
        match &a.assignment {
            Some(x) => v.push(format!(
                "  assigned to {} ({})",
                render_principal(ctx, &x.assigned_to),
                x.purpose
            )),
            None => v.push("  unassigned".to_string()),
        }
        // E6b verify fold (D2): the **count** must not disclose what the line below
        // hides. `ReportStolen` records a `Disputed` claim for every visibility
        // including `Covert`, and the count sat one line above the gated report — so
        // an assignee, or an `asset.*` grantee who is not staff and does not act for
        // the owner, read a covert filing off the arithmetic. A claim the viewer may
        // not be told about is not counted for them either.
        let hidden_claims = usize::from(
            a.stolen
                .as_ref()
                .is_some_and(|r| !may_view_stolen_report(ctx, object, r)),
        );
        v.push(format!(
            "  encumbrances: {}",
            a.encumbrances.len().saturating_sub(hidden_claims)
        ));
        // E6b (B2): the theft report renders **behind its own visibility gate**, so
        // a covert claim is invisible to everyone but its reporter and staff — a
        // thief holding the object reads exactly what an unclaimed asset reads.
        if let Some(report) = a
            .stolen
            .as_ref()
            .filter(|r| may_view_stolen_report(ctx, object, r))
        {
            v.push(format!(
                "  REPORTED STOLEN ({}) by {} at {}{}",
                report.visibility.label(),
                render_principal(ctx, &report.by),
                report.at,
                if report.reason.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", report.reason)
                }
            ));
        }
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
                v.push(format!(
                    "  acquired by transaction #{} at {}",
                    acq.transaction, acq.at
                ));
            }
            v.push(format!(
                "  last journal seq {}, revision {}",
                a.last_journal_seq, a.revision
            ));
            if a.frozen {
                v.push(format!(
                    "  FROZEN ({})",
                    a.frozen_reason.as_deref().unwrap_or("no reason")
                ));
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
        Encumbrance::Lien {
            creditor,
            amount,
            instrument,
        } => {
            format!(
                "lien {} {} to {}",
                money(ctx, instrument, *amount),
                instrument,
                render_principal(ctx, creditor)
            )
        }
        Encumbrance::Disputed { claimant, reason } => {
            format!("disputed by {} ({reason})", render_principal(ctx, claimant))
        }
        Encumbrance::Lease { lessor, contract } => {
            format!(
                "leased from {} (contract {contract})",
                render_principal(ctx, lessor)
            )
        }
        Encumbrance::Collateral { contract } => format!("collateral (contract {contract})"),
    }
}

fn asset_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // `owner=`/`scope=` carry their own `=`, which `ArgShape::EqPair` would eat as the
    // pair separator — read the untouched argument text instead.
    let raw = ctx.args.raw().to_string();
    let (_, opts) = parse_opts(&raw, &["owner", "scope", "after"]);
    // E6b independent verify fold (D10): the walk is bounded on records **visited**
    // and pages from a cursor (`+fence/list`'s shape, D-LBK-5's rule). It read the
    // whole registry before filtering, so on a world with a hundred thousand titles
    // one `@asset/list` was a hundred thousand `may_view_asset` calls whatever the
    // filter said — and the filters ran ahead of the view gate, so the work was done
    // for rows the caller was never going to be shown.
    let after: Option<ObjId> = match opts.get("after") {
        Some(s) => match s.trim().trim_start_matches('#').parse::<u32>() {
            Ok(n) => Some(ObjId(n)),
            Err(_) => return empty_list(ctx, "No assets."),
        },
        None => None,
    };
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
    // The candidates come from the **owner index** when one is named, so the walk is
    // over that principal's own titles rather than the registry; otherwise it is the
    // registry in object order, resumed from the cursor and bounded at
    // [`ASSET_PAGE_VISIT`] records visited.
    let (candidates, last_visited) = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        let snapshot = store.snapshot();
        let walk: Vec<ObjId> = match owner.as_ref() {
            Some(o) => snapshot
                .assets_of(o)
                .filter(|a| after.is_none_or(|c| a.object > c))
                .filter(|a| scope.is_none_or(|s| a.scope == Some(s)))
                .map(|a| a.object)
                .take(ASSET_PAGE_VISIT)
                .collect(),
            None => snapshot
                .assets()
                .filter(|a| after.is_none_or(|c| a.object > c))
                .filter(|a| scope.is_none_or(|s| a.scope == Some(s)))
                .map(|a| a.object)
                .take(ASSET_PAGE_VISIT)
                .collect(),
        };
        let last = walk.last().copied();
        (walk, last)
    };
    let visited = candidates.len();
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
    let more = visited >= ASSET_PAGE_VISIT;
    emit_list(ctx, rows, "No assets.");
    // The cursor is the last record **visited**, not the last one shown: a page that
    // filtered everything out still advances, so a viewer with narrow standing walks
    // the same number of pages a staffer does and the two truncate identically.
    if more && let Some(cursor) = last_visited {
        info(ctx, format!("(more — @asset/list after=#{})", cursor.0));
    }
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
        Err(CmdError::msg(
            "Usage: @vault/create|deposit|withdraw|show|list|freeze|unfreeze|close",
        ))
    }
}

fn vault_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_admin(ctx)?;
    refuse_if_frozen(ctx)?;
    const USAGE: &str = "Usage: @vault/create <name>=container:<obj>|interior:<obj> [owner=<principal>] [scope=<id>]";
    let name = ctx.args.lhs().trim().to_string();
    if name.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let (rest, opts) = parse_opts(rhs, &["owner", "scope"]);
    let spec = rest.trim();
    let backing = if let Some(o) = spec.strip_prefix("container:") {
        CustodyRef::Container(
            match_object(ctx.w, ctx.actor, o).map_err(|_| CmdError::msg("No such object."))?,
        )
    } else if let Some(o) = spec.strip_prefix("interior:") {
        // E3c: legal as soon as SOME registered provider handles the backing
        // kind (`mush_sim::InteriorCustodyProvider`). Whether *this* room is a
        // usable interior is `backing_valid`'s question, below.
        CustodyRef::SpatialInterior(
            match_object(ctx.w, ctx.actor, o).map_err(|_| CmdError::msg("No such object."))?,
        )
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
        return Err(CmdError::msg(
            "That scope does not belong to that organization, or is not active.",
        ));
    }
    // One vault per backing, and never a backing another custody system owns.
    {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        if let Some(v) = store.snapshot().vault_by_backing(&backing) {
            return Err(CmdError::msg(format!(
                "That object already backs vault #{}.",
                v.id
            )));
        }
        let providers = store.providers();
        if !providers.iter().any(|p| p.handles(&backing)) {
            return Err(CmdError::msg(
                if matches!(backing, CustodyRef::SpatialInterior(_)) {
                    "Interior vaults are not available until the interior custody provider is wired."
                } else {
                    "Nothing registered can hold a vault of that kind."
                },
            ));
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
    let id = ctx
        .w
        .module_state_mut::<EconomyStore>()
        .expect("economy store")
        .alloc_vault()
        .ok_or_else(|| {
            CmdError::msg("The vault-id allocator is exhausted; nothing was created.")
        })?;
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
    info(
        ctx,
        format!("Created vault #{id} ({name}) backed by #{}.", backing_obj.0),
    );
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
        Some(other) => match_object(ctx.w, ctx.actor, other)
            .map_err(|_| CmdError::msg("No such destination."))?,
    };
    let plan = ops_plan(
        ctx,
        Vec::new(),
        vec![CustodyOp::Withdraw { object, vault, to }],
    );
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
            let store = ctx
                .w
                .module_state::<EconomyStore>()
                .ok_or_else(|| CmdError::msg("No such vault."))?;
            let v = store
                .snapshot()
                .vault(id)
                .ok_or_else(|| CmdError::msg("No such vault."))?
                .clone();
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
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
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
            format!(
                "#{} {} — owner {} [{:?}]",
                v.id,
                v.name,
                render_principal(ctx, &v.owner),
                v.status
            )
        })
        .collect();
    emit_list(ctx, rows, "No vaults.");
    Ok(())
}

/// Staff doc write (not journaled), like `@asset/unfreeze`.
fn vault_set_status(ctx: &mut Ctx<'_>, status: VaultStatus, verb: &str) -> Result<(), CmdError> {
    require_admin(ctx)?;
    let id = parse_vault_id(ctx.args.lhs()).ok_or_else(|| CmdError::msg("No such vault."))?;
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_none_or(|s| s.snapshot().vault(id).is_none())
    {
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
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No such vault."))?;
        let v = store
            .snapshot()
            .vault(id)
            .ok_or_else(|| CmdError::msg("No such vault."))?;
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

// ---- @hold (E4b1 §4.4) ----------------------------------------------------

fn parse_hold_id(s: &str) -> Option<HoldId> {
    s.trim()
        .trim_start_matches('#')
        .parse::<u64>()
        .ok()
        .map(HoldId)
}
fn parse_obligation_id(s: &str) -> Option<ObligationId> {
    s.trim()
        .trim_start_matches('#')
        .parse::<u64>()
        .ok()
        .map(ObligationId)
}
fn parse_escalation_id(s: &str) -> Option<EscalationId> {
    s.trim()
        .trim_start_matches('#')
        .parse::<u64>()
        .ok()
        .map(EscalationId)
}

/// `<secs>` or `ic:<secs>` — a duration from **now**, resolved onto the clock it
/// names. A deadline always carries its clock (§3.1), so a grace period on an IC
/// schedule can never be compared against wall time by accident.
///
/// The duration must be **positive** (E4b1 verify fold, A7). `expires=0` produced a
/// reservation already past its deadline, and `expires=-60` one a minute past it:
/// both were accepted, both froze the money until the next sweep fire noticed, and
/// both read to the placer as an ordinary hold. A deadline in the past is not a
/// deadline, it is a typo — almost always a `-` that should have been a duration, or
/// a caller subtracting instead of adding.
pub(crate) fn parse_deadline(ctx: &Ctx<'_>, s: &str) -> Option<EconomicDeadline> {
    let (clock, rest, base) = match s.trim().strip_prefix("ic:") {
        Some(rest) => (EconomicClock::WorldIc, rest, ctx.w.game_now_unix()),
        None => (EconomicClock::RealUtc, s.trim(), ctx.w.now()),
    };
    let secs = rest.parse::<i64>().ok().filter(|n| *n > 0)?;
    Some(EconomicDeadline {
        clock,
        at: base.saturating_add(secs),
    })
}

fn cmd_hold(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("place") {
        refuse_if_frozen(ctx)?;
        hold_place(ctx)
    } else if ctx.switches.has("release") {
        refuse_if_frozen(ctx)?;
        hold_release(ctx)
    } else if ctx.switches.has("capture") {
        refuse_if_frozen(ctx)?;
        hold_capture(ctx)
    } else if ctx.switches.has("show") {
        hold_show(ctx)
    } else if ctx.switches.has("list") {
        hold_list(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: @hold/place|release|capture|show|list",
        ))
    }
}

/// `@hold/place <account>=<amount> [for=<principal>] [expires=<secs|ic:<secs>>] [reason]`
///
/// A placement is a **debit-authority** plan (invariant 25: a reservation is a
/// promise to pay) and is therefore subject to the account's `ApprovalRule` exactly
/// like a spend (D-E4-11) — otherwise reserving a fortune and capturing it later
/// would route around the escalation entirely.
fn hold_place(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @hold/place <account>=<amount> [for=<principal>] [expires=<secs|ic:<secs>>] [reason]";
    let account = parse_account(ctx.args.lhs()).ok_or_else(|| CmdError::msg("No such account."))?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let (rest, opts) = parse_opts(rhs, &["for", "expires"]);
    let mut words = rest.split_whitespace();
    let amount = words
        .next()
        .and_then(parse_amount)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    if amount.is_zero() || amount.is_negative() {
        return Err(CmdError::msg("A hold amount must be positive."));
    }
    let beneficiary = match opts.get("for") {
        Some(s) => {
            Some(parse_principal(ctx, s).ok_or_else(|| CmdError::msg("No such principal."))?)
        }
        None => None,
    };
    // A3: `sys:<name>` is a **staff** word, and only for a faucet/sink that really
    // exists. Nobody acts for a system principal (`WorldAuth::acts_for`), so a hold
    // made out to one is capturable only through an internal settlement authority —
    // which makes it a fine thing for a contract to place and a terrible thing for a
    // player to be able to conjure: `@hold/place for=sys:anything` from a player
    // would have frozen their own money into a reservation with no releaser but its
    // placer. The refusal is the ORDINARY unknown-principal text, so a player cannot
    // use it to enumerate which system principals a world has registered.
    if let Some(PrincipalId::System(_)) = &beneficiary {
        let unknown = || CmdError::msg("No such principal.");
        if !is_admin(ctx.w, ctx.actor) {
            return Err(unknown());
        }
        let registered = ctx.w.module_state::<EconomyStore>().is_some_and(|s| {
            s.snapshot()
                .accounts()
                .any(|a| Some(&a.owner) == beneficiary.as_ref())
        });
        if !registered {
            return Err(unknown());
        }
    }
    let expires = match opts.get("expires") {
        Some(s) => Some(parse_deadline(ctx, s).ok_or_else(|| {
            CmdError::msg("Bad expires= (use a POSITIVE number of seconds, or ic:<seconds>).")
        })?),
        None => None,
    };
    let reason: String = words.collect::<Vec<_>>().join(" ");
    let reason = if reason.trim().is_empty() {
        "hold".to_string()
    } else {
        reason
    };
    if reason.len() > crate::MAX_REASON_BYTES {
        return Err(CmdError::msg("That reason is too long."));
    }
    let mut plan = EconomyPlan::new(
        cmd_key(ctx),
        PrincipalId::Object(ctx.actor),
        EconomicCause::Command {
            serial: ctx.w.next_correlation_id(),
        },
        Vec::new(),
    );
    plan.hold_ops = vec![HoldOp::Place {
        account,
        amount,
        beneficiary,
        purpose: HoldPurpose::Manual {
            reason: SmolStr::new(reason),
        },
        expires,
    }];
    let outcome = api::post(ctx, &plan);
    // Name the reservation that was minted: the id is what every later verb needs,
    // and validation assigns it deterministically inside the transaction.
    if let Ok(PostOutcome::Posted(txid)) = &outcome {
        let txid = *txid;
        let placed: Option<HoldId> = ctx.w.module_state::<EconomyStore>().and_then(|s| {
            s.transaction(txid).and_then(|t| {
                t.hold_effects
                    .iter()
                    .find(|e| e.kind == HoldEffectKind::Placed)
                    .map(|e| e.hold.id)
            })
        });
        if let Some(id) = placed {
            info(ctx, format!("Reserved: hold #{id} (transaction #{txid})."));
            info(
                ctx,
                "Those funds are unavailable to any other debit until it is released or captured."
                    .to_string(),
            );
            return Ok(());
        }
    }
    finish_ops(
        ctx,
        NamedResource::Account(account),
        &[],
        outcome,
        "Reserved",
    )
}

/// `@hold/release <id>` — the placer, the beneficiary, anyone with debit authority on
/// the account, or staff. Releasing hands the money back to the very account it was
/// reserved on, which takes nothing from anyone.
fn hold_release(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_hold_id(ctx.args.lhs()).ok_or_else(|| CmdError::msg("No such hold."))?;
    if !may_view_hold(ctx.w, ctx.actor, id) {
        return Err(CmdError::msg("No such hold."));
    }
    refuse_escrow(ctx, id, "released")?;
    refuse_engine_reservation(ctx, id, "released")?;
    let mut plan = EconomyPlan::new(
        cmd_key(ctx),
        PrincipalId::Object(ctx.actor),
        EconomicCause::Command {
            serial: ctx.w.next_correlation_id(),
        },
        Vec::new(),
    );
    plan.hold_ops = vec![HoldOp::Release { hold: id }];
    let outcome = api::post(ctx, &plan);
    finish_ops(ctx, NamedResource::Hold(id), &[], outcome, "Released")
}

/// `@hold/capture <id>=<to account> <amount>` — the **beneficiary's** verb.
///
/// The plan carries the money lines as well as the capture: a capture SPENDS reserved
/// value, so the engine requires the plan's gross debit on the hold's own account to
/// cover it (`CaptureUnbacked` otherwise), and excuses exactly that much of the debit
/// from the live authority check — the placement already proved it.
fn hold_capture(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @hold/capture <id>=<to account> <amount>";
    let id = parse_hold_id(ctx.args.lhs()).ok_or_else(|| CmdError::msg("No such hold."))?;
    if !may_view_hold(ctx.w, ctx.actor, id) {
        return Err(CmdError::msg("No such hold."));
    }
    refuse_escrow(ctx, id, "captured")?;
    refuse_engine_reservation(ctx, id, "captured")?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let mut words = rhs.split_whitespace();
    let to = words
        .next()
        .and_then(parse_account)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let amount = words
        .next()
        .and_then(parse_amount)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    if amount.is_zero() || amount.is_negative() {
        return Err(CmdError::msg("A capture amount must be positive."));
    }
    let from = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No such hold."))?;
        store
            .snapshot()
            .hold(id)
            .ok_or_else(|| CmdError::msg("No such hold."))?
            .account
    };
    if from == to {
        return Err(CmdError::msg("An account cannot pay itself."));
    }
    let neg = amount
        .checked_neg()
        .ok_or_else(|| CmdError::msg("Amount out of range."))?;
    let mut plan = EconomyPlan::new(
        cmd_key(ctx),
        PrincipalId::Object(ctx.actor),
        EconomicCause::Command {
            serial: ctx.w.next_correlation_id(),
        },
        vec![
            RequestedLine {
                account: from,
                delta: neg,
                memo: SmolStr::new_static("capture"),
            },
            RequestedLine {
                account: to,
                delta: amount,
                memo: SmolStr::new_static("capture"),
            },
        ],
    );
    plan.hold_ops = vec![HoldOp::Capture { hold: id, amount }];
    let outcome = api::post(ctx, &plan);
    finish_ops(
        ctx,
        NamedResource::Hold(id),
        &[NamedResource::Account(from), NamedResource::Account(to)],
        outcome,
        "Captured",
    )
}

/// A contract's escrow is the contract's, and only the contract's (§4.8's E4b2
/// handoff). `WorldAuth::authorize_hold` already refuses every op on an
/// `Escrow`-purpose reservation, so this changes no outcome — it replaces
/// "unauthorized" with the sentence that tells the payer what to do instead, for
/// somebody who can already see the reservation. A viewer who cannot see it never
/// reaches this.
fn refuse_escrow(ctx: &Ctx<'_>, id: HoldId, verb: &str) -> Result<(), CmdError> {
    let contract = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().hold(id).cloned())
        .and_then(|h| match h.purpose {
            HoldPurpose::Escrow { contract } => Some(contract),
            HoldPurpose::Reservation { .. } | HoldPurpose::Manual { .. } => None,
        });
    match contract {
        None => Ok(()),
        Some(c) => Err(CmdError::msg(format!(
            "Hold #{id} is contract #{c}'s escrow: it is {verb} by that contract's own settlement              or refund and by nothing else. See @contract/show #{c}."
        ))),
    }
}

/// An **engine-owned reservation** is the engine's, and only the engine's (E7 cut 9
/// independent verify fold, C2; decision E2(a)). `WorldAuth::authorize_hold` already
/// refuses every op on one, so this changes no outcome — it replaces "unauthorized"
/// with the sentence that tells the placer and the beneficiary where the reservation
/// is actually settled. A viewer who cannot see the hold never reaches this.
fn refuse_engine_reservation(ctx: &Ctx<'_>, id: HoldId, verb: &str) -> Result<(), CmdError> {
    let kind = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().hold(id).cloned())
        .filter(|h| crate::is_engine_owned_reservation(&h.purpose))
        .and_then(|h| match h.purpose {
            HoldPurpose::Reservation { kind, .. } => Some(kind),
            HoldPurpose::Escrow { .. } | HoldPurpose::Manual { .. } => None,
        });
    match kind {
        None => Ok(()),
        Some(kind) => Err(CmdError::msg(format!(
            "Hold #{id} is a `{kind}` reservation: it is {verb} by that system's own settlement              and by nothing else."
        ))),
    }
}

/// Render one reservation **through the disclosure filter** (E4c, §6.1 item 5).
///
/// [`may_view_hold`] admits three routes — the account, the placer, and anyone who
/// acts for the **beneficiary** — and only the first of them implies any standing
/// over the account the money is reserved *on*. A payee reading the escrow made out
/// to them was therefore being shown the payer's account id, and a contract id they
/// may have no standing over. Both now go through the same masks the rest of the
/// filter uses ([`mask_account`], [`crate::may_view_contract`]).
/// **A reservation on an account is a fact its viewers may see; the contract behind
/// it is not** (E4c verify fold, D3).
///
/// `@contract/show` omits a `Staff`-visibility contract from everyone outside its
/// class, and `@econ/history` omits its entries — and then `@hold/show` handed the
/// same person the contract id, the escrow beneficiary and a purpose line naming it.
/// One story, one predicate: when the hold is a contract's escrow and the viewer is
/// not staff and is not inside that contract's disclosure class, the *reservation* is
/// rendered and the *contract* is not.
fn escrow_contract_hidden(ctx: &Ctx<'_>, h: &crate::record::Hold) -> bool {
    let HoldPurpose::Escrow { contract } = &h.purpose else {
        return false;
    };
    if is_admin(ctx.w, ctx.actor) {
        return false;
    }
    if !crate::contract_live::may_view_contract(ctx.w, ctx.actor, *contract) {
        return true;
    }
    ctx.w.module_state::<EconomyStore>().is_some_and(|s| {
        s.snapshot().contract(*contract).is_some_and(|c| {
            matches!(
                c.consent.visibility,
                crate::record::EconomicVisibility::Staff
            )
        })
    })
}

fn render_hold(ctx: &Ctx<'_>, h: &crate::record::Hold, staff: bool) -> Vec<String> {
    let status = match &h.status {
        HoldStatus::Held => "held".to_string(),
        HoldStatus::Consumed {
            transaction,
            amount,
        } => {
            format!(
                "captured {} by transaction #{transaction}",
                money_on(ctx, h.account, *amount)
            )
        }
        HoldStatus::Released => "released".to_string(),
        HoldStatus::Expired => "expired".to_string(),
    };
    let purpose = match &h.purpose {
        HoldPurpose::Escrow { contract } => {
            if crate::contract_live::may_view_contract(ctx.w, ctx.actor, *contract) {
                format!("contract #{contract}")
            } else {
                "a contract".to_string()
            }
        }
        HoldPurpose::Reservation { kind, id } => format!("{kind} #{id}"),
        HoldPurpose::Manual { reason } => reason.to_string(),
    };
    let hidden = escrow_contract_hidden(ctx, h);
    let mut lines = vec![format!(
        "hold #{} on {} — {} [{status}]",
        h.id,
        mask_account(ctx, h.account, "an account"),
        money_on(ctx, h.account, h.amount)
    )];
    // D3: no purpose text and no beneficiary for a contract this viewer is outside
    // of — `sys:economy.contract` is itself the answer to "is this a contract?".
    if hidden {
        lines.push("  the reason it is reserved is not yours to see".to_string());
        return lines;
    }
    lines.push(format!("  for {purpose}"));
    if let Some(b) = &h.beneficiary {
        lines.push(format!("  beneficiary {}", render_principal(ctx, b)));
    }
    if let Some(d) = h.expires {
        lines.push(format!("  expires at {} on {:?}", d.at, d.clock));
    }
    if staff {
        lines.push(format!(
            "  placed by {}",
            render_principal(ctx, &h.placed_by)
        ));
        lines.push(format!(
            "  holder {}/{}, journal seq {}, revision {}",
            h.holder.namespace, h.holder.key, h.last_journal_seq, h.revision
        ));
    }
    lines
}

fn hold_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_hold_id(ctx.args.lhs()).ok_or_else(|| CmdError::msg("No such hold."))?;
    if !may_view_hold(ctx.w, ctx.actor, id) {
        return Err(CmdError::msg("No such hold."));
    }
    let staff = is_admin(ctx.w, ctx.actor);
    let lines = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No such hold."))?;
        let snap = store.snapshot();
        let h = snap
            .hold(id)
            .ok_or_else(|| CmdError::msg("No such hold."))?
            .clone();
        render_hold(ctx, &h, staff)
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

/// `@hold/list [account=<id>]`, on E3's list discipline (§4.5): an unviewable or
/// nonexistent `account=` gets the identical "No such account." text, while a
/// viewable account with no reservations gets an empty list — so the two cases a
/// prober wants to distinguish stay indistinguishable, and the honest case is honest.
fn hold_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // `account=` carries its own `=`, which `ArgShape::EqPair` would eat as the pair
    // separator — read the untouched argument text instead (the `@asset/list` shape).
    let raw = ctx.args.raw().to_string();
    let (_, opts) = parse_opts(&raw, &["account"]);
    let filter = match opts.get("account") {
        None => None,
        Some(s) => {
            let id = parse_account(s).ok_or_else(|| CmdError::msg("No such account."))?;
            if !may_view_account(ctx.w, ctx.actor, id) {
                return Err(CmdError::msg("No such account."));
            }
            Some(id)
        }
    };
    let ids: Vec<HoldId> = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        let snap = store.snapshot();
        match filter {
            Some(a) => snap.holds_of(a).map(|h| h.id).collect(),
            None => snap.holds().map(|h| h.id).collect(),
        }
    };
    let rows: Vec<String> = ids
        .into_iter()
        .filter(|id| may_view_hold(ctx.w, ctx.actor, *id))
        .filter_map(|id| {
            let store = ctx.w.module_state::<EconomyStore>()?;
            let snap = store.snapshot();
            let h = snap.hold(id)?;
            let state = match &h.status {
                HoldStatus::Held => "held",
                HoldStatus::Consumed { .. } => "captured",
                HoldStatus::Released => "released",
                HoldStatus::Expired => "expired",
            };
            Some(format!(
                "#{} {} {} [{state}]",
                h.id,
                mask_account(ctx, h.account, "an account"),
                money_on(ctx, h.account, h.amount)
            ))
        })
        .collect();
    emit_list(ctx, rows, "No holds.");
    Ok(())
}

// ---- @obligation (E4b1 §4.4) ----------------------------------------------

fn cmd_obligation(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("create") {
        require_admin(ctx)?;
        refuse_if_frozen(ctx)?;
        obligation_create(ctx)
    } else if ctx.switches.has("show") {
        obligation_show(ctx)
    } else if ctx.switches.has("list") {
        obligation_list(ctx)
    } else if ctx.switches.has("suspend") {
        require_admin(ctx)?;
        obligation_suspend(ctx)
    } else if ctx.switches.has("resume") {
        require_admin(ctx)?;
        obligation_resume(ctx)
    } else if ctx.switches.has("cancel") {
        obligation_cancel(ctx)
    } else if ctx.switches.has("advance") {
        obligation_advance(ctx)
    } else if ctx.switches.has("repoint") {
        require_admin(ctx)?;
        obligation_repoint(ctx)
    } else if ctx.switches.has("reinstate") {
        obligation_reinstate(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: @obligation/create|show|list|suspend|resume|cancel|advance|repoint|reinstate",
        ))
    }
}

/// Fetch an obligation the actor may see, or the nonexistent-id text (§4.5).
fn obligation_of(ctx: &Ctx<'_>, s: &str) -> Result<Obligation, CmdError> {
    let unknown = || CmdError::msg("No such obligation.");
    let id = parse_obligation_id(s).ok_or_else(unknown)?;
    if !may_view_obligation(ctx.w, ctx.actor, id) {
        return Err(unknown());
    }
    let store = ctx.w.module_state::<EconomyStore>().ok_or_else(unknown)?;
    store.snapshot().obligation(id).cloned().ok_or_else(unknown)
}

/// Whether `actor` is the obligation's creditor — the party the money is owed to, and
/// therefore the one party besides staff who may end it, step its manual clock, or
/// reinstate it. The debtor deliberately has none of those powers: a tenant cannot
/// cancel their own rent.
fn is_creditor(ctx: &Ctx<'_>, o: &Obligation) -> bool {
    let me = PrincipalId::Object(ctx.actor);
    if o.creditor == me {
        return true;
    }
    // An org creditor: whoever may spend that compartment speaks for it.
    match ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().account(o.credit_account).cloned())
    {
        Some(a) => match a.scope {
            Some(scope) => {
                api::scope_coherent(ctx.w, Some(scope), &a.owner)
                    && api::holds_capability(ctx.w, ctx.actor, "economy.account.post", scope)
            }
            None => false,
        },
        None => false,
    }
}

/// `@obligation/create <debit acct>=<credit acct> <amount> [every=<secs|ic:<secs>|manual:<secs>>]
/// [start=<secs>] [grace=<n>] [catchup=<n>] [notify]` (staff).
///
/// Everything is checked here that the sweep could otherwise only discover as a
/// refusal every thirty seconds: both accounts exist, are active, and share an
/// instrument; the two sides differ; the amount is positive; and the debit side is
/// **not a registered system faucet** — debiting one creates money from nothing,
/// which is staff-only by fiat (A6) and must never become a standing order the
/// scheduler executes forever.
fn obligation_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @obligation/create <debit acct>=<credit acct> <amount> \
                         [every=<secs|ic:<secs>|manual:<secs>>] [start=<secs>] [grace=<n>] \
                         [catchup=<n>] [notify]";
    let debit = parse_account(ctx.args.lhs()).ok_or_else(|| CmdError::msg("No such account."))?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let (rest, opts) = parse_opts(rhs, &["every", "start", "grace", "catchup"]);
    let mut words = rest.split_whitespace();
    let credit = words
        .next()
        .and_then(parse_account)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let amount = words
        .next()
        .and_then(parse_amount)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let notify_on_success = words.any(|w| w.eq_ignore_ascii_case("notify"));
    if amount.is_zero() || amount.is_negative() {
        return Err(CmdError::msg("An obligation amount must be positive."));
    }
    if debit == credit {
        return Err(CmdError::msg("An account cannot owe itself."));
    }
    let (debtor, creditor) = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        let snap = store.snapshot();
        let d = snap
            .account(debit)
            .ok_or_else(|| CmdError::msg("No such account."))?;
        let c = snap
            .account(credit)
            .ok_or_else(|| CmdError::msg("No such account."))?;
        if d.status != AccountStatus::Active || c.status != AccountStatus::Active {
            return Err(CmdError::msg("Both accounts must be active."));
        }
        if d.instrument != c.instrument {
            return Err(CmdError::msg(
                "Those accounts hold different instruments; there is no implicit exchange.",
            ));
        }
        if matches!(d.owner, PrincipalId::System(_)) {
            return Err(CmdError::msg(
                "A system source/sink cannot be the debit side of an obligation: that would be a \
                 standing mint. Pay from a real account, or grant it with @econ/mint.",
            ));
        }
        (d.owner.clone(), c.owner.clone())
    };
    // The clock, and the first instant a period is due on it.
    let (clock, cadence) = match opts.get("every").map(String::as_str) {
        None => (EconomicClock::RealUtc, None),
        Some(spec) => {
            let (clock, rest) = if let Some(r) = spec.strip_prefix("ic:") {
                (EconomicClock::WorldIc, r)
            } else if let Some(r) = spec.strip_prefix("manual:") {
                (EconomicClock::Manual, r)
            } else {
                (EconomicClock::RealUtc, spec)
            };
            let secs = rest
                .parse::<i64>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| CmdError::msg("every= must be a positive number of seconds."))?;
            (clock, Some(secs))
        }
    };
    let base = match clock {
        EconomicClock::RealUtc => ctx.w.now(),
        EconomicClock::WorldIc => ctx.w.game_now_unix(),
        // A manual schedule starts at position 0 and moves only when someone steps it.
        EconomicClock::Manual | EconomicClock::Cadence(_) => 0,
    };
    let start = match opts.get("start") {
        Some(s) => s
            .parse::<i64>()
            .map_err(|_| CmdError::msg("start= must be a number of seconds."))?,
        // Default: the first period falls due one whole period from now, so creating
        // an obligation never charges instantly for time nobody had notice of.
        None => cadence.unwrap_or(0),
    };
    // S8: `grace=255` was not "255 periods of slack", it was infinite slack — the
    // `missed` counter saturates at 255 and `missed > periods` is then never true, so
    // the obligation could miss forever without ever reaching `Delinquent` or leaving
    // the due pool. Refuse the value that means "never delinquent" rather than
    // silently clamping it: a staffer who typed 255 meant something.
    let grace = match opts.get("grace") {
        Some(s) => {
            let n = s.parse::<u8>().map_err(|_| {
                CmdError::msg(format!("grace= must be 0-{}.", crate::MAX_GRACE_PERIODS))
            })?;
            if n > crate::MAX_GRACE_PERIODS {
                return Err(CmdError::msg(format!(
                    "grace= must be 0-{}: {} would be an allowance that never ends, so the \
                     obligation could never go delinquent.",
                    crate::MAX_GRACE_PERIODS,
                    n
                )));
            }
            n
        }
        None => crate::DEFAULT_GRACE_PERIODS,
    };
    let catchup = match opts.get("catchup") {
        Some(s) => s
            .parse::<u8>()
            .map_err(|_| CmdError::msg("catchup= must be 0-255."))?,
        None => crate::DEFAULT_CATCH_UP,
    };
    let schedule = EconomicSchedule::new(clock, base.saturating_add(start), cadence, catchup);
    let id = {
        let store = ctx
            .w
            .module_state_mut::<EconomyStore>()
            .expect("economy store");
        // B2: the allocator can refuse at the ceiling. Nothing is created, and the
        // refusal says so rather than panicking the world task.
        store.alloc_obligation().ok_or_else(|| {
            CmdError::msg("The obligation-id allocator is exhausted; nothing was created.")
        })?
    };
    let o = Obligation {
        id,
        debtor,
        creditor,
        debit_account: debit,
        credit_account: credit,
        amount,
        schedule,
        grace: GracePolicy::new(grace),
        notify_on_success,
        status: ObligationStatus::Active,
        cause: ObligationCause::Staff {
            reason: SmolStr::new_static("@obligation/create"),
        },
        // M1: a command never mints. A faucet-funded obligation is a standing mint
        // and only `mush_economy::create_system_obligation` — the ruleset's own
        // entry, under the configured cap — can create one.
        minted_by: None,
        periods_settled: 0,
        history: Vec::new(),
        settled_total: 0,
        missed_total: 0,
        forgiven_total: 0,
        suspensions: std::collections::BTreeSet::new(),
        revision: 1,
    };
    api::put_obligation(ctx, o);
    let meta = ctx
        .w
        .module_state::<EconomyStore>()
        .map(EconomyStore::meta_doc);
    if let Some(meta) = meta {
        api::persist_meta(ctx.out, meta);
    }
    info(
        ctx,
        format!("Obligation #{id} created: #{debit} owes #{credit} each period."),
    );
    info(
        ctx,
        "The sweep attempts it on its own clock. A downtime backlog is forgiven, never charged."
            .to_string(),
    );
    // S9: a documented limit, said out loud at the moment it starts applying.
    //
    // An `ApprovalRule` is a PER-TRANSACTION escalation, and every period of an
    // obligation is its own transaction — so a per-period amount under the threshold
    // never escalates however many years it runs (the sub-threshold drip), while one
    // above it cannot settle at all, because the sweep has nobody to collect
    // approvals from and pages instead. Neither is silently fixable: a budget
    // primitive is E5's, and making the sweep collect approvals would park rent
    // forever waiting for a human. What the staffer can have is the warning, at the
    // moment they create the thing.
    let ruled = ctx.w.module_state::<EconomyStore>().is_some_and(|s| {
        s.snapshot()
            .account(debit)
            .is_some_and(|a| a.approval.is_some())
    });
    if ruled {
        info(
            ctx,
            format!(
                "Note: account #{debit} carries an approval rule, and a standing obligation is \
                 not subject to it the way a payment is. Each period is its own transaction: a \
                 per-period amount under the threshold never escalates however long it runs, and \
                 one over it cannot settle at all (the sweep cannot collect approvals)."
            ),
        );
    }
    // A container doc, not money: the ordinary durable gate (§4.2).
    ctx.gate_on_durable_ack();
    Ok(())
}

fn render_schedule(s: &EconomicSchedule) -> String {
    let every = match s.cadence {
        None => "one-shot".to_string(),
        Some(c) => format!("every {c}"),
    };
    format!(
        "{every} on {:?} (next due {}, catch-up {})",
        s.clock, s.next_due, s.maximum_catch_up
    )
}

fn render_status(o: &Obligation) -> String {
    match &o.status {
        ObligationStatus::Active => "active".to_string(),
        ObligationStatus::Grace { missed, deadline } => {
            format!(
                "in grace ({missed} missed; allowance ends {} on {:?})",
                deadline.at, deadline.clock
            )
        }
        ObligationStatus::Delinquent => "delinquent".to_string(),
        ObligationStatus::Suspended { reason } => {
            let word = match reason {
                SuspendReason::Staff { reason } => format!("suspended ({reason})"),
                SuspendReason::Escalated(id) => format!("suspended pending staff — alert #{id}"),
                SuspendReason::Held { key } => format!("suspended — held by {key}"),
            };
            // Every holder, when there is more than the one the reason names
            // (look-back fold, D-LBK-1): the obligation returns to the due pool only
            // when all of them have released.
            let holders = o.holders();
            if holders.len() > 1 {
                let all: Vec<&str> = holders.iter().map(SuspensionKey::as_str).collect();
                format!("{word} [holders: {}]", all.join(", "))
            } else {
                word
            }
        }
        ObligationStatus::Satisfied => "satisfied".to_string(),
        ObligationStatus::Cancelled => "cancelled".to_string(),
    }
}

/// **The instrument an obligation is denominated in** (E6a step 1 verify fold, M8):
/// either side's, since a post is per-instrument zero-sum and the two must agree.
///
/// It is the fallback a debt amount renders in when the viewer's standing comes from
/// the *other* side of the record — `may_view_obligation` admits a creditor who may
/// not view the debit account at all, and D2's gate would otherwise leave them
/// reading their own rent as a bare minor value.
fn obligation_instrument(ctx: &Ctx<'_>, o: &crate::obligation::Obligation) -> Option<InstrumentId> {
    let store = ctx.w.module_state::<EconomyStore>()?;
    let snap = store.snapshot();
    snap.account(o.debit_account)
        .or_else(|| snap.account(o.credit_account))
        .map(|a| a.instrument.clone())
}

fn obligation_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let o = obligation_of(ctx, ctx.args.lhs())?;
    let staff = is_admin(ctx.w, ctx.actor);
    let instrument = obligation_instrument(ctx, &o);
    let mut lines = vec![
        // D2 (E4c verify fold): the ids go through the same mask every other render
        // uses. `may_view_obligation` admits either party and anyone who may view
        // EITHER account, so a debtor was being handed the id of a creditor account
        // they have no standing over — a handle for `@econ/history` and `@econ/pay`.
        // The amount stays: it is the viewer's own side of the debt.
        format!(
            "obligation #{} — {} owes {} {} [{}]",
            o.id,
            mask_account(ctx, o.debit_account, "an account"),
            mask_account(ctx, o.credit_account, "an account"),
            money_on_or(ctx, o.debit_account, o.amount, instrument.as_ref()),
            render_status(&o)
        ),
        format!("  {}", render_schedule(&o.schedule)),
        format!(
            "  {} settled, {} missed, {} forgiven; {} period(s) closed",
            o.settled_total, o.missed_total, o.forgiven_total, o.periods_settled
        ),
    ];
    if staff {
        lines.push(format!("  debtor {}", render_principal(ctx, &o.debtor)));
        lines.push(format!("  creditor {}", render_principal(ctx, &o.creditor)));
        lines.push(format!(
            "  grace {} period(s), notify_on_success {}, revision {}",
            o.grace.periods, o.notify_on_success, o.revision
        ));
    }
    // The bounded tail (§3.4): the journal is the real history, this is the summary.
    for e in o.history.iter().rev().take(5) {
        lines.push(format!("  {}", render_event(e)));
    }
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn render_event(e: &ObligationEvent) -> String {
    match e {
        ObligationEvent::Settled {
            period,
            at,
            transaction,
        } => match transaction {
            Some(t) => format!("period {period} settled at {at} (transaction #{t})"),
            None => format!("period {period} settled at {at}"),
        },
        ObligationEvent::Missed { period, at } => format!("period {period} missed at {at}"),
        ObligationEvent::Forgiven {
            periods,
            from,
            to,
            cause,
            at,
        } => {
            format!("{periods} period(s) forgiven ({cause:?}) at {at} [{from}..{to})")
        }
        ObligationEvent::Escalated { id, at } => format!("escalated as alert #{id} at {at}"),
        ObligationEvent::Resumed { at } => format!("resumed at {at}"),
        ObligationEvent::Held { key, at } => format!("held by {key} at {at}"),
        ObligationEvent::Released { key, at } => format!("released by {key} at {at}"),
    }
}

fn obligation_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let ids: Vec<ObligationId> = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        store.snapshot().obligations().map(|o| o.id).collect()
    };
    let rows: Vec<String> = ids
        .into_iter()
        .filter(|id| may_view_obligation(ctx.w, ctx.actor, *id))
        .filter_map(|id| {
            let store = ctx.w.module_state::<EconomyStore>()?;
            let snap = store.snapshot();
            let o = snap.obligation(id)?;
            Some(format!(
                "#{} {} → {} {} [{}]",
                o.id,
                mask_account(ctx, o.debit_account, "an account"),
                mask_account(ctx, o.credit_account, "an account"),
                money_on_or(
                    ctx,
                    o.debit_account,
                    o.amount,
                    obligation_instrument(ctx, o).as_ref()
                ),
                render_status(o)
            ))
        })
        .collect();
    emit_list(ctx, rows, "No obligations.");
    Ok(())
}

/// Record a status change plus its history entry, keeping the bounded tail bounded.
pub(crate) fn transition(
    o: &Obligation,
    status: ObligationStatus,
    event: Option<ObligationEvent>,
) -> Obligation {
    let mut next = o.clone();
    next.status = status;
    next.revision += 1;
    if let Some(event) = event {
        if next.history.len() >= crate::MAX_OBLIGATION_HISTORY {
            next.history.remove(0);
        }
        next.history.push(event);
    }
    next
}

fn obligation_suspend(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let o = obligation_of(ctx, ctx.args.lhs())?;
    let reason = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("staff");
    if reason.len() > crate::MAX_REASON_BYTES {
        return Err(CmdError::msg("That reason is too long."));
    }
    if !o.status.in_due_pool() {
        return Err(CmdError::msg(format!(
            "Obligation #{} is not running.",
            o.id
        )));
    }
    let next = crate::obligation::hold(
        &o,
        SuspendReason::Staff {
            reason: SmolStr::new(reason),
        },
        ctx.w.now(),
    );
    let id = next.id;
    api::put_obligation(ctx, next);
    info(
        ctx,
        format!("Obligation #{id} suspended; the sweep will not attempt it."),
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `@obligation/resume <id>` (staff) — put a **staff-suspended** obligation back in
/// the due pool, forgiving the gap the suspension left.
///
/// Two things the E4b1 build got wrong, both fixed here:
///
/// - **An escalated suspension is not staff's to lift with this verb** (E4b1 verify
///   fold, A6). `Suspended { Escalated(id) }` means an alert is open and a human has
///   not answered it; `resume` cleared the status and left the alert open, so the
///   record still said "a decision is pending" while the obligation quietly went
///   back to attempting the very settlement that had failed. `@econ/ack <id>` is the
///   verb that answers an alert *and* resumes its subject, and it is one command.
/// - **The old notice promised forgiveness nothing delivered** (S5). It said the
///   suspended periods would be forgiven by the next sweep — but nothing re-anchored
///   the schedule, so the next sweep saw the whole gap as due and charged
///   `maximum_catch_up` of it. The schedule is re-anchored here, exactly as
///   `@obligation/reinstate` does, and the forgiveness is **recorded** with
///   [`crate::ForgiveCause::Suspension`] so the obligation's own history says what
///   happened instead of the periods vanishing.
fn obligation_resume(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let o = obligation_of(ctx, ctx.args.lhs())?;
    if !matches!(o.status, ObligationStatus::Suspended { .. }) {
        return Err(CmdError::msg(format!(
            "Obligation #{} is not suspended.",
            o.id
        )));
    }
    // Look-back fold, D-LBK-1: the obligation carries every holder, and this verb
    // lifts exactly one of them — staff's own. The ladder's key is `@econ/ack`'s, and
    // a layered module's key is that module's; refusing both here is what keeps
    // "returns to the due pool only when the set is empty" true.
    let holders = o.holders();
    if let Some(alert) = holders.iter().find_map(SuspensionKey::escalation_id) {
        return Err(CmdError::msg(format!(
            "Obligation #{} is suspended by alert #{alert}, which is still open. Answer the alert \
             instead — `@econ/alerts {alert}` shows the recommended fix, and `@econ/ack {alert}` \
             resumes it once that is done.",
            o.id
        )));
    }
    if let Some(held) = holders.iter().find(|k| k.is_module_hold()) {
        return Err(CmdError::msg(format!(
            "Obligation #{} is held out of the due pool by `{held}`, which a module holds; it \
             returns to the due pool when that hold is released, not by this verb.",
            o.id
        )));
    }
    let before = o.forgiven_total;
    let id = o.id;
    let returned = api::resume_obligation(ctx, id, &SuspensionKey::staff())
        .map_err(|e| CmdError::msg(e.to_string()))?;
    let after = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().obligation(id).cloned());
    let (forgiven, due_at) = after.map_or((0, 0), |o| {
        (o.forgiven_total.saturating_sub(before), o.schedule.next_due)
    });
    if returned {
        info(
            ctx,
            format!("Obligation #{id} resumed; the next period falls due at {due_at}."),
        );
    } else {
        info(
            ctx,
            format!("Obligation #{id}: the staff hold is released; it is still held."),
        );
    }
    if forgiven > 0 {
        info(
            ctx,
            format!(
                "{forgiven} period(s) fell due while it was suspended; they are forgiven and \
                 recorded, not charged."
            ),
        );
    } else {
        info(
            ctx,
            "No period fell due while it was suspended.".to_string(),
        );
    }
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `@obligation/cancel <id>` — staff, or the creditor (the party owed may forgive a
/// debt; the party owing may not).
fn obligation_cancel(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let o = obligation_of(ctx, ctx.args.lhs())?;
    if !is_admin(ctx.w, ctx.actor) && !is_creditor(ctx, &o) {
        return Err(CmdError::msg("No such obligation."));
    }
    if matches!(o.status, ObligationStatus::Cancelled) {
        return Err(CmdError::msg(format!(
            "Obligation #{} is already cancelled.",
            o.id
        )));
    }
    let next = transition(&o, ObligationStatus::Cancelled, None);
    let id = next.id;
    api::put_obligation(ctx, next);
    info(ctx, format!("Obligation #{id} cancelled."));
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `@obligation/advance <id>` — step a **Manual**-clock obligation forward one
/// period. Staff or the creditor: advancing a manual clock is what makes the next
/// period fall due, so it is the act of charging, not of paying.
///
/// **One period per invocation, and never past an unsettled one** (E4b1 verify fold,
/// A4). The E4b1 build added a whole cadence to `manual_now` on every call with no
/// ceiling at all, so six impatient invocations before the next sweep left six
/// periods due at once — and the sweep then charged `maximum_catch_up` of them in one
/// fire and forgave the rest, which is a lot of somebody's money for a command whose
/// help says "one period". The clock now advances at most to `next_due`, and a second
/// call before the sweep has settled that period is a **no-op with a notice** rather
/// than a silent multiplier: a decision per invocation, which is what a manual clock
/// is for.
fn obligation_advance(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let o = obligation_of(ctx, ctx.args.lhs())?;
    if !is_admin(ctx.w, ctx.actor) && !is_creditor(ctx, &o) {
        return Err(CmdError::msg("No such obligation."));
    }
    if o.schedule.clock != EconomicClock::Manual {
        return Err(CmdError::msg(
            "That obligation is not on a manual clock; it advances on its own.",
        ));
    }
    // Stepping the clock of a delinquent, suspended, cancelled or satisfied
    // obligation charges nothing (the sweep will not attempt it) and quietly builds a
    // backlog that lands the moment somebody reinstates it.
    if !o.status.in_due_pool() {
        return Err(CmdError::msg(format!(
            "Obligation #{} is {} — its clock does not move until it is back in the due pool.",
            o.id,
            render_status(&o)
        )));
    }
    if o.schedule.manual_now >= o.schedule.next_due {
        info(
            ctx,
            format!(
                "Obligation #{}'s clock is already at {} and a period is due at {} but not yet \
                 settled. Nothing moved; the next sweep will settle it.",
                o.id, o.schedule.manual_now, o.schedule.next_due
            ),
        );
        return Ok(());
    }
    let step = o.schedule.cadence.unwrap_or(1).max(1);
    let mut next = o.clone();
    next.schedule.manual_now = next
        .schedule
        .manual_now
        .saturating_add(step)
        .min(next.schedule.next_due);
    next.revision += 1;
    let (id, position) = (next.id, next.schedule.manual_now);
    api::put_obligation(ctx, next);
    info(
        ctx,
        format!("Obligation #{id}'s manual clock is now at {position}; one period is due."),
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `@obligation/repoint <id>=<debit|credit> <account>` (staff) — the fix an
/// escalation recommends when one side's account has gone. It never lowers
/// `periods_settled`, so a repointed obligation cannot be made to re-charge history.
fn obligation_repoint(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @obligation/repoint <id>=<debit|credit> <account>";
    let o = obligation_of(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let mut words = rhs.split_whitespace();
    let side = words.next().unwrap_or("").to_ascii_lowercase();
    let account = words
        .next()
        .and_then(parse_account)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let (owner, instrument) = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        let snap = store.snapshot();
        let a = snap
            .account(account)
            .ok_or_else(|| CmdError::msg("No such account."))?;
        if a.status != AccountStatus::Active {
            return Err(account_refusal(ctx, account, "That account is not active."));
        }
        (a.owner.clone(), a.instrument.clone())
    };
    let existing = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        let snap = store.snapshot();
        let other = if side == "credit" {
            o.debit_account
        } else {
            o.credit_account
        };
        snap.account(other).map(|a| a.instrument.clone())
    };
    if existing.is_some_and(|i| i != instrument) {
        return Err(CmdError::msg(
            "That account holds a different instrument; there is no implicit exchange.",
        ));
    }
    let mut next = o.clone();
    match side.as_str() {
        "credit" => {
            next.credit_account = account;
            next.creditor = owner;
        }
        "debit" => {
            if matches!(owner, PrincipalId::System(_)) {
                return Err(CmdError::msg(
                    "A system source/sink cannot be the debit side of an obligation.",
                ));
            }
            next.debit_account = account;
            next.debtor = owner;
        }
        _ => return Err(CmdError::msg(USAGE)),
    }
    if next.debit_account == next.credit_account {
        return Err(CmdError::msg("An account cannot owe itself."));
    }
    next.revision += 1;
    let id = next.id;
    api::put_obligation(ctx, next);
    info(
        ctx,
        format!("Obligation #{id} repointed: its {side} side is now account #{account}."),
    );
    info(
        ctx,
        "Nothing was charged, and no closed period was reopened.".to_string(),
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `@obligation/reinstate <id>` — staff or the creditor.
///
/// E4a deliberately gives `Delinquent` **no automatic recovery**: `advance` records a
/// late payment but leaves the status alone, so an obligation that ran out of grace
/// stays out of the due pool until a person puts it back (the E4a handoff). This is
/// that person's verb. The schedule is re-anchored to the first period due **after
/// now**, so reinstating never charges for the time it spent delinquent.
fn obligation_reinstate(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let o = obligation_of(ctx, ctx.args.lhs())?;
    if !is_admin(ctx.w, ctx.actor) && !is_creditor(ctx, &o) {
        return Err(CmdError::msg("No such obligation."));
    }
    if !matches!(o.status, ObligationStatus::Delinquent) {
        return Err(CmdError::msg(format!(
            "Obligation #{} is not delinquent.",
            o.id
        )));
    }
    // Look-back fold, D-LBK-1: a holder's key on a delinquent obligation keeps it out
    // of the pool through a reinstatement too.
    if let Some(held) = o.holders().iter().next() {
        return Err(CmdError::msg(format!(
            "Obligation #{} is held out of the due pool by `{held}`; it cannot be reinstated \
             until that hold is released.",
            o.id
        )));
    }
    // A5: reinstating puts the obligation straight back in the due pool, so both
    // sides must still be able to carry it. Without this the very next sweep found a
    // closed or frozen account, refused, paged a staffer, and suspended the record
    // again — a round trip through the escalation ladder to learn something the
    // command could have said at once, and with the fix (`@obligation/repoint`)
    // already in hand.
    {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        let snap = store.snapshot();
        for (label, id) in [("debit", o.debit_account), ("credit", o.credit_account)] {
            match snap.account(id) {
                Some(a) if a.status == AccountStatus::Active => {}
                _ => {
                    return Err(account_refusal(
                        ctx,
                        id,
                        format!(
                            "Obligation #{}'s {label} account #{id} is not active; repoint it \
                             (@obligation/repoint #{}={label} <account id>) before reinstating.",
                            o.id, o.id
                        ),
                    ));
                }
            }
        }
    }
    let now = ctx.w.now();
    let clock_now = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        Clocks::new(now, ctx.w.game_now_unix()).now_on(
            o.schedule.clock,
            store.snapshot(),
            o.schedule.manual_now,
        )
    };
    let mut next = transition(
        &o,
        ObligationStatus::Active,
        Some(ObligationEvent::Resumed { at: now }),
    );
    if let Some(clock_now) = clock_now {
        next.schedule = crate::clock::reanchor(&next.schedule, clock_now);
    }
    let id = next.id;
    let due = next.schedule.next_due;
    api::put_obligation(ctx, next);
    info(
        ctx,
        format!("Obligation #{id} reinstated; the next period falls due at {due}."),
    );
    info(
        ctx,
        "The periods it spent delinquent are not charged.".to_string(),
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

// ---- @econ/alerts | @econ/ack (the ladder's staff surface, §4.6) -----------

/// `@econ/alerts` — the open escalations, newest first, capped (staff).
///
/// Each line carries the diagnosis, and `@econ/alerts <id>` prints the recommended
/// commands with their arguments already filled in: the staffer is going to paste
/// this into an LLM anyway, so the alert is written to already be the answer.
fn econ_alerts(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let arg = ctx.args.lhs().trim().to_string();
    if !arg.is_empty() {
        let id = parse_escalation_id(&arg).ok_or_else(|| CmdError::msg("No such alert."))?;
        let record = {
            let store = ctx
                .w
                .module_state::<EconomyStore>()
                .ok_or_else(|| CmdError::msg("No economy."))?;
            store
                .escalation(id)
                .cloned()
                .ok_or_else(|| CmdError::msg("No such alert."))?
        };
        info(ctx, record.render());
        for r in &record.recommended {
            info(ctx, format!("  → {r}"));
        }
        if record.is_open() {
            info(ctx, format!("  → @econ/ack #{id}   (once it is handled)"));
        }
        return Ok(());
    }
    let mut rows: Vec<(i64, String)> = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        store
            .escalations()
            .filter(|e| e.is_open())
            .map(|e| (e.opened_at, e.render()))
            .collect()
    };
    // Newest first (§4.6); the id breaks ties within one second.
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    let rows: Vec<String> = rows.into_iter().map(|(_, r)| r).collect();
    emit_list(ctx, rows, "No open economy alerts.");
    // E6b (C16): the bounded refusal telemetry, beside the ladder rather than in
    // it. It opens no escalation and suspends nothing — a probe is not an incident
    // — but staff can see the shape of one instead of having to infer it.
    let tallies: Vec<(ObjId, crate::module::RefusalTally)> = ctx
        .w
        .module_state::<EconomyStore>()
        .map(|s| s.refusal_tallies())
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, t)| t.hidden > 0)
        .take(crate::module::MAX_REFUSAL_ROWS)
        .collect();
    if !tallies.is_empty() {
        info(
            ctx,
            "Refusals (existence-hiding first, this session only):".to_string(),
        );
        for (who, tally) in tallies {
            let line = format!(
                "  {} — {} hidden, {} other (first {}, last {})",
                display_name(ctx.w, who),
                tally.hidden,
                tally.other,
                tally.first_at,
                tally.last_at
            );
            info(ctx, line);
        }
    }
    Ok(())
}

/// `@econ/ack <id>` — mark one alert handled and **resume its subject where that is
/// the recommendation** (§4.6).
///
/// Acknowledging an obligation's alert puts it back in the due pool; the periods that
/// fell due while it was suspended are forgiven by the next sweep, never charged. An
/// alert about the whole economy is a different thing: acknowledging it does NOT
/// clear the freeze, because the freeze is a decision someone makes with
/// `@econ/unfreeze` after looking at the drift.
fn econ_ack(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_escalation_id(ctx.args.lhs()).ok_or_else(|| CmdError::msg("No such alert."))?;
    let mut record = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        store
            .escalation(id)
            .cloned()
            .ok_or_else(|| CmdError::msg("No such alert."))?
    };
    if !record.is_open() {
        return Err(CmdError::msg(format!(
            "Alert #{id} is already acknowledged."
        )));
    }
    let now = ctx.w.now();
    record.acked_by = Some(ctx.actor);
    record.acked_at = Some(now);
    record.revision += 1;
    let subject = record.subject;
    api::put_escalation(ctx, record);

    // Look-back fold, D-LBK-1: acknowledging releases the ladder's own key and
    // nothing else. With another holder left (a staff suspension, a layered module's
    // hold) the obligation stays out of the pool and the line says so.
    let mut resumed = None;
    if let EscalationSubject::Obligation(oid) = subject {
        let key = SuspensionKey::escalation(id);
        let held = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().obligation(oid).map(|o| o.held_by(&key)))
            .unwrap_or(false);
        if held {
            match api::resume_obligation(ctx, oid, &key) {
                Ok(returned) => resumed = Some((oid, returned)),
                Err(error) => {
                    tracing::warn!(obligation = oid.0, %error, "economy: ack could not release")
                }
            }
        }
    }
    info(ctx, format!("Alert #{id} acknowledged."));
    match resumed {
        Some((oid, true)) => {
            info(ctx, format!("Obligation #{oid} is back in the due pool."));
            info(
                ctx,
                "The periods that fell due while it was suspended were forgiven as they fell due, \
                 not charged."
                    .to_string(),
            );
        }
        Some((oid, false)) => {
            let holders: Vec<String> = ctx
                .w
                .module_state::<EconomyStore>()
                .and_then(|s| {
                    s.snapshot()
                        .obligation(oid)
                        .map(|o| o.holders().iter().map(ToString::to_string).collect())
                })
                .unwrap_or_default();
            info(
                ctx,
                format!(
                    "Obligation #{oid} is still out of the due pool: it is held by {}.",
                    holders.join(", ")
                ),
            );
        }
        None => match subject {
            EscalationSubject::Economy => info(
                ctx,
                "That does not clear the whole-economy freeze: read the boot log, then \
                 @econ/unfreeze."
                    .to_string(),
            ),
            // E4b2: a contract's alert is not a thing acknowledgement resumes. A
            // disputed contract's escrow stays frozen until somebody decides which
            // way it goes, and that decision is its own verb.
            EscalationSubject::Contract(cid) => info(
                ctx,
                format!(
                    "Contract #{cid} is still disputed: acknowledging does not release its \
                     escrow. @contract/resolve #{cid}=settle|refund is the decision."
                ),
            ),
            // E5b: housing's own alerts are acknowledged here like any other, and
            // acknowledging one changes nothing about the home — D-E5-1 is the whole
            // point of the page. The record's `recommended` carries the verbs.
            EscalationSubject::HousingProperty(pid) => info(
                ctx,
                format!(
                    "Housing property #{pid} is still withdrawn: acknowledging does not \
                     reinstate it or end any tenancy on it (@property/show {pid})."
                ),
            ),
            EscalationSubject::HousingTenancy(tid) => info(
                ctx,
                format!(
                    "Tenancy #{tid} is unchanged: nothing ends a home automatically. \
                     @tenancy/resolve {tid}=extend|end|waive is the decision."
                ),
            ),
            // Look-back fold, D-LBK-1: a quarantined room is released only by
            // re-validation, never by an acknowledgement.
            EscalationSubject::HousingRoom(room) => info(
                ctx,
                format!(
                    "Room #{room} is still under housing review: acknowledging does not lift a \
                     quarantine (@housing/quarantine show)."
                ),
            ),
            // SP1 (D-LBK-7): a declared cadence's page is a decision about which
            // side — the configuration or the live clock — is right. Acknowledging
            // rewrites neither.
            EscalationSubject::DeclaredCadence(cid) => info(
                ctx,
                format!(
                    "Cadence #{cid} is unchanged: acknowledging does not rewrite a live clock \
                     or the configuration that declares it (@econ/cadence show #{cid})."
                ),
            ),
            // R8: a dropped announcement is a chronicle problem, never a money one —
            // acknowledging it is the whole remedy, and the record says which line
            // was lost.
            EscalationSubject::DroppedPage => info(
                ctx,
                "That announcement was never delivered; the money it was about moved \
                 regardless. Nothing else is stopped."
                    .to_string(),
            ),
            EscalationSubject::Obligation(_) | EscalationSubject::Hold(_) => {}
        },
    }
    ctx.gate_on_durable_ack();
    Ok(())
}
