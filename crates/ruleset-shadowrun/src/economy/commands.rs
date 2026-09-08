//! The E6a verb surface (08f §3.4).
//!
//! | verb | who | what |
//! |---|---|---|
//! | `+run` | player | `/list`, `/show`, `/post`, `/split`, `/apply`, `/crew`, `/withdraw` |
//! | `+lifestyle` | player | view; `/set <tier>`, `/pay` |
//! | `+nuyen` | player | the derived balance after approval; the sheet field before |
//! | `@sr` | staff | `/economy [show]`, `/clock`, `/migrate [dry\|commit] [<who>]`, `/profile <who>` |
//!
//! Three disclosure rules run through all of them, and each is the economy's own:
//!
//! - **Existence hiding, always.** A run, party or profile the actor may not view is
//!   `"No such run."` / `"No such party."` — identical for nonexistent and
//!   unviewable (08a decision 13). Every account render goes through
//!   [`mush_economy::money_on`], which asks `may_view_account` once for every call
//!   site and renders an unviewable account exactly like an unknown one (D2).
//! - **`+run/show` renders the board's own notice, never the terms** (M5, G7): the
//!   name, the state, the window, the *kinds* of condition, and the Johnson when the
//!   poster announced one. The contract's own notice is the economy's to render, and
//!   `contract_notice` is what renders it.
//! - **The sheet number is not money after approval** (D-E6-1). `+nuyen` shows the
//!   account balance once the character is approved, the sheet field before; the
//!   number itself is only ever shown to its owner and to staff.

use std::sync::Arc;

use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ModError, ObjId, Registrar, Role, match_object, perm,
};
use mush_economy::{
    AccountId, Amount, AttestorSet, EconomyStore, MAX_OUTCOMES, NamedResource, may_view_account,
    parse_account, parse_opts, parse_principal,
};
use mush_faction::{FactionId, FactionState};
use mush_org::{OrgScopeId, PrincipalId};
use smol_str::SmolStr;

use crate::Sr5Cmd;
use crate::economy::board::{self, NO_SUCH_RUN, PostRequest, RunRoute, RunSplit, SplitSlot};
use crate::economy::chargen::{MigrationVerdict, approval_stamp};
use crate::economy::config::{
    FENCE_PAGE, FENCE_PAGE_VISIT, LifestyleTier, MAX_RUN_OUTCOMES, MAX_RUN_SLOTS, SR_MONTH,
    SrEconomyConfig,
};
use crate::economy::fence_record::{FenceId, FenceQuote, FenceSettlement, QuoteId};
use crate::economy::lifestyle::{self, LifestyleError};
use crate::economy::record::{BoardVisibility, RunId, RunState, RunTemplate};
use crate::economy::store::SrEconomyStore;

/// Wire the E6a surface. Every verb registers whatever the switches say — an
/// `enabled = false` server answers "not enabled on this server", it does not hide
/// the verb (E5c deviation 1).
pub(crate) fn register(reg: &mut Registrar<'_>, cmd: Arc<Sr5Cmd>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+run".into(),
            switches: vec![
                "list".into(),
                "show".into(),
                "post".into(),
                "split".into(),
                "apply".into(),
                "crew".into(),
                "withdraw".into(),
            ],
            shape: ArgShape::EqPair,
            help: "+run/list [public|faction|mine] [after=<id>]  --  the run board, newest \
                   first, one bounded page at a time; the page names its own cursor when there \
                   is more.\n\
                   +run/show <id>  --  a listing's notice (never its terms).\n\
                   +run/post <name>=<pay> [outcomes=success,failure] [scope=<org scope>] \
                   [account=<id>] [johnson=<who>] [fixer=<who>] [fee=<amount>] \
                   [advance=<amount>] [adjudicator=<who>] [slots=<n>] \
                   [board=public|faction:<f>|invite:<party>] [days=<n>]  --  draft, offer and \
                   escrow in one act. You are the payer and the issuer; the escrow is placed \
                   from your own account, so nothing is listed that is not funded.\n\
                   +run/split <id>=fee|advance <amount>  --  edit a slot while still listed.\n\
                   +run/apply <id>=<party>  --  your party offers itself (leader only).\n\
                   +run/crew <id>=accept|decline [<party>]  --  the payer's answer.\n\
                   +run/withdraw <id>  --  take a listing down; the escrow is refunded."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| run_cmd(&cmd, ctx))
        },
    )?;

    // E6b (B9): the fence. A new primary verb, entered in the rule-1 allowlist in
    // the same commit as `legacy-post-image` with its future family `sr:fence`.
    reg.command(
        CommandSpec {
            name: "+fence".into(),
            switches: vec![
                "list".into(),
                "show".into(),
                "quote".into(),
                "accept".into(),
                "deliver".into(),
            ],
            shape: ArgShape::EqPair,
            help: "+fence/list  --  who is buying, and your own open quotes.\n\
                   +fence/show <quote>  --  one quote and the whole of its arithmetic.\n\
                   +fence/quote <fence>=<object> [immediate|consign]  --  a deterministic offer \
                   for something you are carrying. Immediate pays less and settles on delivery; \
                   a consignment pays more and matures after a stated delay. The price comes \
                   from the catalogue baseline, the settlement rate and the fence's declared \
                   demand, and from nothing else — a fence never pays for what it cannot see.\n\
                   +fence/accept <quote>  --  the fence escrows its payment. You then accept the \
                   contract (@contract/accept), and only then deliver.\n\
                   +fence/deliver <quote>  --  hand the goods into the fence's vault. Refused \
                   until the deal is in force: the money is committed before the goods move.\n\
                   A fence buys CUSTODY, never title. Your title record and any dispute on it \
                   survive the sale untouched."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(fence_cmd),
    )?;

    reg.command(
        CommandSpec {
            name: "+lifestyle".into(),
            switches: vec!["set".into(), "pay".into()],
            shape: ArgShape::EqPair,
            help: "+lifestyle  --  your tier, its cost, and what you owe.\n\
                   +lifestyle/set <tier>  --  street|squatter|low|middle|high|luxury. Applies \
                   from the NEXT period, and is refused while you are in arrears.\n\
                   +lifestyle/pay  --  pay the arrears, then reinstate."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| lifestyle_cmd(ctx)),
    )?;

    // ---- E7r2: P0c — `+nuyen` is its OWN registration group, `ReadOnly` ----
    //
    // E7 residue pass 1's fold added the class tripwire (allowlist category vs the
    // class the production composition publishes) and it found `+nuyen` declaring
    // `read-only` while `ShadowrunRuleset::register`'s module-wide
    // `classify_commands(LegacyPostImage)` published it as a whole-document
    // post-image writer. The fold's note called that "the dangerous direction" —
    // the allowlist asserting a verb persists nothing while the composition says it
    // does — and pass 2 fixes it on the side the tree makes true: `nuyen_cmd` is a
    // pure render (`match_object`, `perm::can_examine`, `may_view_account`, `say`)
    // and writes nothing at all, so the honest repair is the CLASS, not the stanza.
    // Moving the stanza to `legacy-post-image` would have needed a fabricated
    // "future family" for a write path that does not exist.
    //
    // K8.5's discipline, per group: classify before this group's own `reg.command`,
    // and restore `LegacyPostImage` immediately after so every later registration in
    // this module keeps the class its own documents ride.
    reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
    reg.command(
        CommandSpec {
            name: "+nuyen".into(),
            shape: ArgShape::Raw,
            help: "+nuyen [<who>]  --  your nuyen balance (the account after approval, the \
                   sheet field before). Someone else's only if you are staff."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| nuyen_cmd(&cmd, ctx))
        },
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);

    reg.command(
        CommandSpec {
            name: "@sr".into(),
            switches: vec![
                "economy".into(),
                "clock".into(),
                "migrate".into(),
                "profile".into(),
                // E6b (B9): register a vendor organization as a fence.
                "fence".into(),
            ],
            shape: ArgShape::EqPair,
            min_role: Role::Royalty,
            help: "@sr/economy [show]  --  the effective [shadowrun.economy] block.\n\
                   @sr/clock  --  the sr.month cadence clock (declared in [economy.cadences]; \
                   this verb shows it, it never creates one).\n\
                   @sr/migrate dry|commit [<who>]  --  mint an approved character's sheet nuyen \
                   once from system:sr.migration. Idempotent; a re-run reports the first result.\n\
                   @sr/profile <who>  --  one character's economic profile.
                   @sr/fence <org>=<vault> [name=<text>] [demand=<permille>]  --  register a                    vendor organization as a fence. The org must be named in                    [shadowrun.economy].vendor_orgs (composition data: it is what lets a fence                    accept a deal with no officer connected) and must own the vault."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| sr_cmd(&cmd, ctx))
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// shared
// ---------------------------------------------------------------------------

fn config(ctx: &Ctx<'_>) -> SrEconomyConfig {
    ctx.w
        .module_state::<SrEconomyStore>()
        .map_or_else(SrEconomyConfig::default, |s| s.config().clone())
}

fn require_enabled(cfg: &SrEconomyConfig) -> Result<(), CmdError> {
    if cfg.enabled {
        Ok(())
    } else {
        Err(CmdError::msg(
            "The Shadowrun economic layer is not enabled on this server.",
        ))
    }
}

fn say(ctx: &mut Ctx<'_>, line: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "sr.economy", line.into());
}

fn parse_run_id(s: &str) -> Option<RunId> {
    s.trim()
        .trim_start_matches('#')
        .parse::<u64>()
        .ok()
        .map(RunId)
}

fn parse_u64(s: &str) -> Option<u64> {
    s.trim().trim_start_matches('#').parse::<u64>().ok()
}

fn amount_of(s: &str) -> Option<Amount> {
    s.trim().parse::<i64>().ok().map(Amount::from_minor)
}

// ---------------------------------------------------------------------------
// +run
// ---------------------------------------------------------------------------

fn run_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let cfg = config(ctx);
    let sw = ctx.switches.clone();
    if sw.has("post") {
        require_enabled(&cfg)?;
        return run_post(ctx, &cfg);
    }
    if sw.has("split") {
        require_enabled(&cfg)?;
        return run_split(ctx);
    }
    if sw.has("apply") {
        require_enabled(&cfg)?;
        return run_apply(ctx);
    }
    if sw.has("crew") {
        require_enabled(&cfg)?;
        return run_crew(ctx);
    }
    if sw.has("withdraw") {
        require_enabled(&cfg)?;
        return run_withdraw(ctx);
    }
    if sw.has("show") {
        require_enabled(&cfg)?;
        return run_show(ctx);
    }
    let _ = cmd;
    require_enabled(&cfg)?;
    run_list(ctx, &cfg)
}

fn run_list(ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: +run/list [public|faction|mine] [after=<id>]";
    let lhs = ctx.args.lhs().trim().to_ascii_lowercase();
    let (rest, opts) = parse_opts(&lhs, &["after"]);
    let filter = match rest.trim() {
        "" | "all" => None,
        w @ ("public" | "faction" | "mine") => Some(w.to_string()),
        _ => return Err(CmdError::msg(USAGE)),
    };
    // D1: the cursor is how a bounded walk reaches the rest of the board. It names a
    // listing id, which is not a secret — an id the actor may not view simply does not
    // appear in the page it starts.
    let after = match opts.get("after") {
        Some(s) => Some(parse_run_id(s).ok_or_else(|| CmdError::msg(USAGE))?),
        None => None,
    };
    let page = cfg.board_cap.min(crate::economy::config::BOARD_PAGE);
    let found = board::visible(ctx.w, ctx.actor, filter.as_deref(), page, after);
    if found.ids.is_empty() {
        match found.next {
            Some(next) => say(
                ctx,
                format!(
                    "Nothing for you in the last {} listing(s). More board: +run/list after={next}",
                    found.visited
                ),
            ),
            None => say(ctx, "The board has nothing for you."),
        }
        return Ok(());
    }
    say(ctx, format!("Run board ({} listing(s)):", found.ids.len()));
    let next = found.next;
    for id in found.ids {
        let Some(r) = ctx
            .w
            .module_state::<SrEconomyStore>()
            .and_then(|s| s.run(id).cloned())
        else {
            continue;
        };
        let line = format!(
            "  #{}  {}  [{}]  {}  slots {}",
            r.id,
            r.notice.name,
            r.state.word(),
            r.board.word(),
            r.slots
        );
        say(ctx, line);
    }
    if let Some(next) = next {
        say(ctx, format!("  … more board: +run/list after={next}"));
    }
    Ok(())
}

fn run_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_run_id(ctx.args.lhs()).ok_or_else(|| CmdError::msg(NO_SUCH_RUN))?;
    let Some(r) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.run(id).cloned())
        .filter(|r| board::may_view(ctx.w, ctx.actor, r))
    else {
        // D1: a closed listing's document is deleted, so the grace ring is what keeps
        // "it just closed" from reading as "it never existed" to somebody who was
        // looking at it a minute ago. It says only that, and it says it to anybody —
        // there is nothing left to hide, and the ring holds ids and not records.
        if ctx
            .w
            .module_state::<SrEconomyStore>()
            .is_some_and(|s| s.recently_closed(id))
        {
            say(ctx, format!("Run #{id}: closed."));
            return Ok(());
        }
        return Err(CmdError::msg(NO_SUCH_RUN));
    };
    for line in notice_lines(ctx, &r) {
        say(ctx, line);
    }
    Ok(())
}

/// The **board's own notice** (M5, G7): name, state, window, condition kinds, and the
/// Johnson when announced. Never the terms; the amounts are rendered only to somebody
/// who may already view the funding account.
fn notice_lines(ctx: &Ctx<'_>, r: &RunTemplate) -> Vec<String> {
    let mut out = vec![
        format!("Run #{}: {}", r.id, r.notice.name),
        format!(
            "  state: {}   board: {}   slots: {}",
            r.state.word(),
            r.board.word(),
            r.slots
        ),
        format!(
            "  window: closes at {} (posted {})",
            r.expires_at, r.posted_at
        ),
        format!(
            "  settles on: {}",
            if r.notice.kinds.is_empty() {
                "attest".to_string()
            } else {
                r.notice.kinds.join(", ")
            }
        ),
        format!("  outcomes: {}", r.outcomes.join(", ")),
    ];
    if let Some(j) = &r.notice.johnson {
        out.push(format!(
            "  Johnson: {}",
            mush_economy::render_principal(ctx, j)
        ));
    }
    // The money is the contract's, and it is shown only to somebody the *account's*
    // own view gate already admits (D2). Everybody else is told nothing at all —
    // never a placeholder.
    if may_view_account(ctx.w, ctx.actor, r.account) {
        out.push(format!(
            "  pay: {}   fixer fee: {}   advance: {}",
            mush_economy::money_on(ctx, r.account, r.pay),
            mush_economy::money_on(ctx, r.account, r.fee),
            mush_economy::money_on(ctx, r.account, r.advance)
        ));
    }
    // The contract's own notice, rendered by the economy so the ruleset never keeps a
    // third copy of the rule (E5b-D36/D37).
    if let Some(c) = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().contract(r.contract).cloned())
        && mush_economy::may_view_contract(ctx.w, ctx.actor, r.contract)
    {
        out.extend(
            mush_economy::contract_notice(ctx, &c)
                .into_iter()
                .map(|l| format!("  {l}")),
        );
    }
    out
}

/// The option keys `+run/post` understands.
const POST_KEYS: [&str; 13] = [
    "outcomes",
    "scope",
    "account",
    "johnson",
    "fixer",
    "fee",
    "advance",
    "adjudicator",
    "slots",
    "board",
    "days",
    // E6b (B8): the other three settlement routes.
    "route",
    "gate",
];

// ---- E6b: B8 — parsing the four routes ------------------------------------

/// Parse one route spec: `attest`, `canon:<kind>:<source>`, `before:<unix>`,
/// `after:<row>:<secs>` or `fact:<provider>:<kind>:<id>`.
///
/// The **source is the remainder**, deliberately: a seal's source is `run:{id}`,
/// which carries a colon of its own, and splitting it off by position would have
/// made the one source E6b actually produces unspellable.
fn parse_route_spec(spec: &str, rows: &[SmolStr]) -> Result<RunRoute, CmdError> {
    const USAGE: &str = "A route is attest | canon:<kind>:<source> | before:<unix> | \
                         after:<row>:<secs> | fact:<provider>:<kind>:<id>.";
    let spec = spec.trim();
    if spec.eq_ignore_ascii_case("attest") {
        return Ok(RunRoute::Attest);
    }
    let (word, rest) = spec.split_once(':').ok_or_else(|| CmdError::msg(USAGE))?;
    match word.trim().to_ascii_lowercase().as_str() {
        "canon" => {
            let (kind, source) = rest.split_once(':').ok_or_else(|| CmdError::msg(USAGE))?;
            if kind.trim().is_empty() || source.trim().is_empty() {
                return Err(CmdError::msg(USAGE));
            }
            Ok(RunRoute::Canon {
                kind: SmolStr::new(kind.trim()),
                source: SmolStr::new(source.trim()),
            })
        }
        "before" => Ok(RunRoute::Before {
            at: rest
                .trim()
                .parse::<i64>()
                .map_err(|_| CmdError::msg(USAGE))?,
        }),
        "after" => {
            let (row, secs) = rest.split_once(':').ok_or_else(|| CmdError::msg(USAGE))?;
            let anchor = row_index(row.trim(), rows)?;
            let delay = secs
                .trim()
                .parse::<i64>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| CmdError::msg(USAGE))?;
            Ok(RunRoute::After { anchor, delay })
        }
        "fact" => {
            let (provider, rest) = rest.split_once(':').ok_or_else(|| CmdError::msg(USAGE))?;
            let (kind, id) = rest.split_once(':').ok_or_else(|| CmdError::msg(USAGE))?;
            if provider.trim().is_empty() || kind.trim().is_empty() || id.trim().is_empty() {
                return Err(CmdError::msg(USAGE));
            }
            Ok(RunRoute::Fact {
                provider: SmolStr::new(provider.trim()),
                kind: SmolStr::new(kind.trim()),
                id: SmolStr::new(id.trim()),
            })
        }
        _ => Err(CmdError::msg(USAGE)),
    }
}

/// Resolve an outcome row by name or by index.
fn row_index(word: &str, rows: &[SmolStr]) -> Result<usize, CmdError> {
    if let Some(i) = rows.iter().position(|n| n.eq_ignore_ascii_case(word)) {
        return Ok(i);
    }
    word.parse::<usize>()
        .ok()
        .filter(|i| *i < rows.len())
        .ok_or_else(|| CmdError::msg("That run has no such outcome row."))
}

/// Parse `route=<row>:<spec>|<row>:<spec>…` into one route per outcome row,
/// defaulting every unnamed row to `attest` (E6a's behaviour exactly).
fn parse_routes(raw: Option<&str>, rows: &[SmolStr]) -> Result<Vec<RunRoute>, CmdError> {
    let mut routes: Vec<RunRoute> = vec![RunRoute::Attest; rows.len()];
    let Some(raw) = raw else { return Ok(routes) };
    for part in raw.split('|').map(str::trim).filter(|p| !p.is_empty()) {
        let (row, spec) = part.split_once(':').ok_or_else(|| {
            CmdError::msg("Usage: route=<row>:<attest|canon:…|before:…|after:…|fact:…>")
        })?;
        let i = row_index(row.trim(), rows)?;
        let route = parse_route_spec(spec, rows)?;
        // An `after` naming its own row would be a condition waiting on itself; the
        // economy's cycle check refuses it at offer, and saying so here is clearer.
        if matches!(&route, RunRoute::After { anchor, .. } if *anchor == i) {
            return Err(CmdError::msg("An after route cannot wait on its own row."));
        }
        routes[i] = route;
    }
    Ok(routes)
}

/// Parse `gate=<spec>|<spec>…` — the conditions **every** row must also satisfy.
fn parse_gates(raw: Option<&str>, rows: &[SmolStr]) -> Result<Vec<RunRoute>, CmdError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let mut out: Vec<RunRoute> = Vec::new();
    for part in raw.split('|').map(str::trim).filter(|p| !p.is_empty()) {
        let route = parse_route_spec(part, rows)?;
        // E6b independent verify fold (M3): **a gate is not a row, so it is never
        // canon.** A `canon:` gate could not be satisfied by anything: `+run/post`
        // rewrote its `run:self` to a bare `run:{next}`, while the producer's seal
        // always emits `run:{contract}#{row}` (M5, because one source per run made
        // one seal fill every row) — so the two could never match, and a crewed run
        // carrying such a gate simply froze its escrow until the deadline with no
        // refusal anywhere saying why. Canon evidence is per row by construction;
        // refusing it here is the honest answer, at the moment a human can still fix
        // the listing.
        if matches!(route, RunRoute::Canon { .. }) {
            return Err(CmdError::msg(
                "A gate cannot be a canon route: a sealed outcome answers one outcome row, so it \
                 belongs in route=<row>:canon:… and not in gate=.",
            ));
        }
        out.push(route);
        if out.len() + rows.len() > mush_economy::MAX_CONDITIONS {
            return Err(CmdError::msg("That run names too many conditions."));
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_lines)]
fn run_post(ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: +run/post <name>=<pay> [outcomes=a,b] [scope=<id>] \
                         [account=<id>] [johnson=<who>] [fixer=<who>] [fee=<n>] [advance=<n>] \
                         [adjudicator=<who>] [slots=<n>] [board=public|faction:<f>|invite:<p>] \
                         [days=<n>]";
    let name = ctx.args.lhs().trim().to_string();
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    if name.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    let (rest, opts) = parse_opts(rhs, &POST_KEYS);
    let pay = amount_of(&rest).ok_or_else(|| CmdError::msg(USAGE))?;

    let scope = match opts.get("scope") {
        Some(s) => Some(
            parse_u64(s)
                .map(OrgScopeId)
                .ok_or_else(|| CmdError::msg("No such scope."))?,
        ),
        None => None,
    };
    // §9's partial-switch table: each row says what it stops, in the off-mode words.
    if scope.is_some() && !cfg.organization_run_offers {
        return Err(CmdError::msg(
            "Organizations do not post runs on this server.",
        ));
    }
    if scope.is_none() && !cfg.player_run_offers {
        return Err(CmdError::msg("Players do not post runs on this server."));
    }
    // The funding account: named, or the poster's own profile account, or the org's
    // account on the named scope.
    let account = resolve_account(ctx, scope, opts.get("account").map(String::as_str))?;
    // **The payer is derived from the account's own owner** (answer 4: the accountable
    // escrow principal is a fact about the wallet), never named by the poster.
    let principal = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().account(account).map(|a| a.owner.clone()))
        .ok_or_else(|| refuse_account(ctx, account))?;
    // §11 q7: the poster may not name an org account they cannot view.
    if !may_view_account(ctx.w, ctx.actor, account) {
        return Err(refuse_account(ctx, account));
    }
    // D-E6-4 / M4: a `System` account is refused **at post**, decided on the account
    // owner before any draft exists, in that account's own hiding text.
    if matches!(principal, PrincipalId::System(_)) {
        return Err(CmdError::msg("No such account."));
    }

    let names = outcome_names(opts.get("outcomes").map(String::as_str))?;
    let slots = match opts.get("slots") {
        Some(s) => s
            .trim()
            .parse::<u8>()
            .ok()
            .filter(|n| (1..=MAX_RUN_SLOTS).contains(n))
            .ok_or_else(|| CmdError::msg(format!("Slots must be 1..={MAX_RUN_SLOTS}.")))?,
        None => 3,
    };
    let fee = match opts.get("fee") {
        Some(s) => {
            amount_of(s).ok_or_else(|| CmdError::msg("A fee is a whole number of nuyen."))?
        }
        None => Amount::ZERO,
    };
    let advance = match opts.get("advance") {
        Some(s) => {
            amount_of(s).ok_or_else(|| CmdError::msg("An advance is a whole number of nuyen."))?
        }
        None => Amount::ZERO,
    };
    let fixer = match opts.get("fixer") {
        Some(s) => Some(parse_principal(ctx, s).ok_or_else(|| CmdError::msg("No such fixer."))?),
        None => None,
    };
    // D5: **the fee-row rule is asked first, before the account.** It is a fact about
    // the command and not about the person, so its answer must not depend on whether
    // the named principal is banked — and it used to: `+run/post X=1 fixer=<who>` with
    // no `fee=` answered "that fixer has no nuyen account" for an unbanked target and
    // "a fixer needs a fee row" for a banked one. That was a free probe, costing no
    // escrow and leaving no listing, for whether any named character holds an
    // `sr.nuyen` account.
    if fixer.is_some() && fee.minor() <= 0 {
        return Err(CmdError::msg(
            "A fixer needs a fee row (fee=<amount>); an unnamed remainder never reaches them.",
        ));
    }
    // A fixer is `roles.author`/`representative` with a fee row, and NEVER the issuer
    // (M1, D-E6-19): the fee is paid to their account, the run is still the payer's.
    //
    // The account's own absence renders as the **unresolvable-fixer** text (D5), so
    // "there is no such person" and "that person has no wallet" are one answer. The
    // actionable message survives only for somebody the account's own gate already
    // admits — which, by `may_view_account`, is its owner and staff.
    let fixer_account = match &fixer {
        Some(p) => Some(principal_account(ctx, p).ok_or_else(|| {
            CmdError::msg(
                if fixer_is_self(ctx, p) || crate::economy::is_staff(ctx.w, ctx.actor) {
                    "That fixer has no nuyen account to be paid a fee into."
                } else {
                    "No such fixer."
                },
            )
        })?),
        None => None,
    };
    let johnson = match opts.get("johnson") {
        Some(s) => Some(parse_principal(ctx, s).ok_or_else(|| CmdError::msg("No such Johnson."))?),
        None => None,
    };
    // The roles rule (answer 22, §3.3): the representative is never the payer.
    if let Some(j) = &johnson
        && *j == principal
    {
        return Err(CmdError::msg(
            "The Johnson is the visible representative; on your own run that is you, so leave it \
             unset.",
        ));
    }
    // D-E6-20 + the E6a verify fold (M4, answer 22 as amended: distinct wherever a
    // conflict of interest exists): `adjudicator=` defaults to `party:<payer>`; a
    // NAMED one is never the payer, never the poster, never the **fixer** and never
    // the **Johnson**. The fixer is paid out of every crewed row and the Johnson is
    // the payer's own visible representative — an adjudicator who is either of them
    // is the authority that ends the run standing on one side of it.
    let (adjudicator, adjudicator_named) = match opts.get("adjudicator") {
        Some(s) => {
            let p = parse_principal(ctx, s).ok_or_else(|| CmdError::msg("No such adjudicator."))?;
            if p == principal || p == PrincipalId::Object(ctx.actor) {
                return Err(CmdError::msg(
                    "The adjudicator settles the run and is never a party to it — not the payer, \
                     and not you.",
                ));
            }
            if fixer.as_ref() == Some(&p) {
                return Err(CmdError::msg(
                    "The adjudicator settles the run and the fixer is paid by it; they cannot be \
                     the same person.",
                ));
            }
            if johnson.as_ref() == Some(&p) {
                return Err(CmdError::msg(
                    "The adjudicator settles the run and the Johnson represents the payer in it; \
                     they cannot be the same person.",
                ));
            }
            (AttestorSet::Party(p), true)
        }
        None => (AttestorSet::Party(principal.clone()), false),
    };
    // E6b (B8): the four routes. Every unnamed row stays `attest`, so a command
    // that names none behaves exactly as E6a's did. A **canon** route is refused at
    // offer unless its kind is a producer-declared outcome kind and it names a
    // source, so `route=success:canon:scene.outcome` (no source) never lists.
    let routes = parse_routes(opts.get("route").map(String::as_str), &names)?;
    let gate_routes = parse_gates(opts.get("gate").map(String::as_str), &names)?;
    // E6b independent verify fold (M2): **a routed run does not leave a row on the
    // payer's word.** Every unnamed row defaults to `attest`, and B8 deliberately
    // names *no* person adjudicator on a routed run — so on a deterministic surface
    // `consent.adjudicator` is `None`, `@contract/attest`'s `None` arm falls back to
    // the condition's own set, and that set is `AttestorSet::Party(payer)`. The payer
    // could therefore attest the one row nobody routed and settle the run on their
    // own word, while `AdjudicatorShape::Impersonal` told the crew check there was no
    // person to conflict with and the board read the run as deterministic.
    //
    // So: once any row is routed, either every row is routed, or `adjudicator=` names
    // somebody — the very thing whose consent the crew agreed to at acceptance.
    if routes
        .iter()
        .chain(gate_routes.iter())
        .any(RunRoute::is_deterministic)
        && routes.iter().any(|r| !r.is_deterministic())
        && !adjudicator_named
    {
        return Err(CmdError::msg(
            "This run routes some outcomes and leaves others on an attestation, and it names no \
             adjudicator — so the payer would settle the unrouted row on their own word. Route \
             every row, or name an adjudicator (adjudicator=<who>).",
        ));
    }
    // The gates sit after the per-row conditions, so their indexes are known before
    // the surface exists.
    let gates: Vec<usize> = (names.len()..names.len() + gate_routes.len()).collect();
    let board = parse_board(ctx, opts.get("board").map(String::as_str))?;
    let days = match opts.get("days") {
        Some(s) => s
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| CmdError::msg("days must be a positive number."))?,
        None => cfg.board_ttl() / 86_400,
    };
    let ttl = days.saturating_mul(86_400).min(cfg.board_ttl()).max(60);
    let expires_at = ctx.w.now().saturating_add(ttl);

    let req = PostRequest {
        name: SmolStr::new(&name),
        scope,
        account,
        principal,
        split: RunSplit {
            pay,
            fee,
            advance,
            payer_account: account,
            fixer_account,
            names,
            gates,
        },
        adjudicator,
        adjudicator_named,
        johnson,
        fixer,
        slots,
        board,
        expires_at,
        routes,
        gates: gate_routes,
    };
    match board::post(ctx, cfg, req) {
        Ok(id) => {
            say(
                ctx,
                format!(
                    "Run #{id} is listed and its escrow is placed. Nothing is on the board that \
                     is not funded."
                ),
            );
            Ok(())
        }
        Err((e, named)) => Err(board::refuse_post(ctx, &e, &named)),
    }
}

/// The funding account: named, the poster's own, or the org's on the named scope.
fn resolve_account(
    ctx: &Ctx<'_>,
    scope: Option<OrgScopeId>,
    named: Option<&str>,
) -> Result<AccountId, CmdError> {
    if let Some(s) = named {
        return parse_account(s).ok_or_else(|| CmdError::msg("No such account."));
    }
    let instrument = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.instrument.clone())
        .ok_or_else(|| CmdError::msg("The nuyen instrument is not registered."))?;
    let Some(scope) = scope else {
        return ctx
            .w
            .module_state::<SrEconomyStore>()
            .and_then(|s| s.profile(ctx.actor).and_then(|p| p.account))
            .or_else(|| crate::economy::boot::personal_account(ctx, ctx.actor, &instrument))
            .ok_or_else(|| CmdError::msg("You have no nuyen account to fund a run from."));
    };
    // D3: the candidate scan is filtered through the account's **own** view gate
    // before it is counted. Counting first made the "more than one nuyen account"
    // line an oracle about the shape of a compartment the actor may not look into —
    // three distinguishable outcomes where the hiding rule allows one. An outsider now
    // reads "No such account." for a two-account compartment and for a scope that does
    // not exist alike.
    let mut found: Vec<AccountId> = ctx
        .w
        .module_state::<EconomyStore>()
        .map(|s| {
            s.snapshot()
                .accounts()
                .filter(|a| a.scope == Some(scope) && a.instrument == instrument)
                .map(|a| a.id)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    found.retain(|id| may_view_account(ctx.w, ctx.actor, *id));
    found.sort_unstable();
    match found.as_slice() {
        [one] => Ok(*one),
        _ => Err(CmdError::msg("No such account.")),
    }
}

/// `p`'s live personal `sr.nuyen` account.
///
/// Indexed by owner (D13) and **`Active` only** (R4) — the same two rules
/// [`crate::economy::boot::personal_account`] follows, for the same reasons. The
/// disclosure rule is the *caller's*: the fee row must be paid into a real wallet, so
/// this resolves one, and `run_post` decides which words a `None` gets (D5).
fn principal_account(ctx: &Ctx<'_>, p: &PrincipalId) -> Option<AccountId> {
    let instrument = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.instrument.clone())?;
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .accounts_of(p)
        .find(|a| {
            a.instrument == instrument
                && a.scope.is_none()
                && a.status == mush_economy::AccountStatus::Active
        })
        .map(|a| a.id)
}

/// Whether the named fixer is the actor themselves — the one case where the
/// actionable "they have no wallet" message tells the actor something about their own
/// account rather than about somebody else's (D5).
fn fixer_is_self(ctx: &Ctx<'_>, p: &PrincipalId) -> bool {
    *p == PrincipalId::Object(ctx.actor)
}

fn refuse_account(ctx: &mut Ctx<'_>, id: AccountId) -> CmdError {
    mush_economy::refuse_named(
        ctx,
        NamedResource::Account(id),
        [],
        "No such account.".to_string(),
    )
}

fn outcome_names(raw: Option<&str>) -> Result<Vec<SmolStr>, CmdError> {
    let raw = raw.unwrap_or("success,failure");
    let names: Vec<SmolStr> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(SmolStr::new)
        .collect();
    if names.is_empty() {
        return Err(CmdError::msg(
            "A run always carries an outcome table, even with one row.",
        ));
    }
    if names.len() > MAX_RUN_OUTCOMES.min(MAX_OUTCOMES) {
        return Err(CmdError::msg(format!(
            "A run carries at most {MAX_RUN_OUTCOMES} outcome rows."
        )));
    }
    // D2: the row name is frozen into the consent surface, copied into the board doc
    // and broadcast by `+run/show`. The count and the duplicates were capped and the
    // **bytes** were not, so `outcomes=<4 kB>` was a permanent 4 kB string on the
    // contract and on the board. (The economy refuses one at `check_structure` too —
    // this is the message a poster can act on.)
    if names
        .iter()
        .any(|n| n.len() > mush_economy::MAX_OUTCOME_NAME_BYTES)
    {
        return Err(CmdError::msg(format!(
            "An outcome row name is at most {} bytes.",
            mush_economy::MAX_OUTCOME_NAME_BYTES
        )));
    }
    // `resolve_outcome` refuses two satisfied rows, so two rows with the same name
    // would be a table nobody can read even though the economy would accept it.
    for (i, n) in names.iter().enumerate() {
        if names.iter().skip(i + 1).any(|m| m == n) {
            return Err(CmdError::msg("Two outcome rows have the same name."));
        }
    }
    Ok(names)
}

fn parse_board(ctx: &Ctx<'_>, raw: Option<&str>) -> Result<BoardVisibility, CmdError> {
    let Some(raw) = raw else {
        return Ok(BoardVisibility::Public);
    };
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("public") {
        return Ok(BoardVisibility::Public);
    }
    if let Some(rest) = raw.strip_prefix("faction:") {
        let f = resolve_faction(ctx, rest).ok_or_else(|| CmdError::msg("No such faction."))?;
        return Ok(BoardVisibility::Faction(f));
    }
    if let Some(rest) = raw.strip_prefix("invite:") {
        let party = parse_u64(rest).ok_or_else(|| CmdError::msg(mush_party::NO_SUCH_PARTY))?;
        if !mush_party::may_view(ctx.w, ctx.actor, mush_party::PartyId(party)) {
            return Err(CmdError::msg(mush_party::NO_SUCH_PARTY));
        }
        return Ok(BoardVisibility::Invitation { party });
    }
    Err(CmdError::msg(
        "board= is public, faction:<f>, or invite:<party>.",
    ))
}

/// `board=faction:<id|name>` — through `mush_faction`'s **own** listing predicate on
/// both branches (E6a verify fold, D4).
///
/// The numeric branch used to ask `contains_key` alone, so a hidden faction that
/// answered "No such faction." by name answered *yes* by id: the id was a
/// straightforward existence oracle for exactly the records the name lookup hides.
fn resolve_faction(ctx: &Ctx<'_>, raw: &str) -> Option<FactionId> {
    let st = ctx.w.module_state::<FactionState>()?;
    let raw = raw.trim();
    let staff = crate::economy::is_staff(ctx.w, ctx.actor);
    if let Some(n) = parse_u64(raw) {
        let id = FactionId(n);
        if let Some(f) = st.factions.get(&id)
            && st.faction_listed(ctx.actor, staff, f)
        {
            return Some(id);
        }
    }
    st.faction_by_name(raw)
        .filter(|f| st.faction_listed(ctx.actor, staff, f))
        .map(|f| f.id)
}

fn run_split(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: +run/split <id>=fee|advance <amount>";
    let id = parse_run_id(ctx.args.lhs()).ok_or_else(|| CmdError::msg(NO_SUCH_RUN))?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let mut words = rhs.split_whitespace();
    let slot = match words.next().map(str::to_ascii_lowercase).as_deref() {
        Some("fee") => SplitSlot::Fee,
        Some("advance") => SplitSlot::Advance,
        _ => return Err(CmdError::msg(USAGE)),
    };
    let amount = words
        .next()
        .and_then(amount_of)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let contract = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.run(id).map(|r| r.contract));
    match board::split(ctx, id, slot, amount) {
        Ok(line) => {
            say(ctx, line);
            Ok(())
        }
        Err(e) => Err(board::refuse_board(ctx, contract, &e)),
    }
}

fn run_apply(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: +run/apply <id>=<party>";
    let id = parse_run_id(ctx.args.lhs()).ok_or_else(|| CmdError::msg(NO_SUCH_RUN))?;
    let party = ctx
        .args
        .rhs()
        .and_then(parse_u64)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    // The leader offers their party — the party subsystem's own view rule decides
    // whether this actor may even see it, and its leader field decides who may offer.
    let leads = ctx
        .w
        .module_state::<mush_party::PartyStore>()
        .and_then(|s| s.party(mush_party::PartyId(party)).map(|p| p.leader))
        == Some(ctx.actor);
    if !mush_party::may_view(ctx.w, ctx.actor, mush_party::PartyId(party)) {
        return Err(CmdError::msg(mush_party::NO_SUCH_PARTY));
    }
    if !leads {
        return Err(CmdError::msg("Only a party's leader offers it for a run."));
    }
    let contract = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.run(id).map(|r| r.contract));
    match board::apply(ctx, id, party) {
        Ok(line) => {
            say(ctx, line);
            Ok(())
        }
        Err(e) => Err(board::refuse_board(ctx, contract, &e)),
    }
}

fn run_crew(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: +run/crew <id>=accept|decline [<party>]";
    let id = parse_run_id(ctx.args.lhs()).ok_or_else(|| CmdError::msg(NO_SUCH_RUN))?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let mut words = rhs.split_whitespace();
    let verb = words.next().map(str::to_ascii_lowercase);
    let party = words.next().and_then(parse_u64);
    let contract = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.run(id).map(|r| r.contract));
    let outcome = match verb.as_deref() {
        Some("accept") => board::crew_accept(ctx, id, party),
        Some("decline") => board::crew_decline(ctx, id),
        _ => return Err(CmdError::msg(USAGE)),
    };
    match outcome {
        Ok(line) => {
            say(ctx, line);
            Ok(())
        }
        Err(e) => Err(board::refuse_board(ctx, contract, &e)),
    }
}

fn run_withdraw(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_run_id(ctx.args.lhs()).ok_or_else(|| CmdError::msg(NO_SUCH_RUN))?;
    let contract = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.run(id).map(|r| r.contract));
    match board::withdraw(ctx, id) {
        Ok(line) => {
            say(ctx, line);
            Ok(())
        }
        Err(e) => Err(board::refuse_board(ctx, contract, &e)),
    }
}

// ---------------------------------------------------------------------------
// +lifestyle
// ---------------------------------------------------------------------------

fn lifestyle_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let cfg = config(ctx);
    require_enabled(&cfg)?;
    let sw = ctx.switches.clone();
    if sw.has("set") {
        let raw = ctx.args.lhs().trim().to_string();
        let tier = LifestyleTier::parse(&raw).ok_or_else(|| {
            CmdError::msg("Usage: +lifestyle/set street|squatter|low|middle|high|luxury")
        })?;
        let who = ctx.actor;
        return match lifestyle::set_tier(ctx, &cfg, who, tier) {
            Ok(Some(line)) => {
                say(ctx, line);
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(e) => Err(refuse_lifestyle(&e)),
        };
    }
    if sw.has("pay") {
        let who = ctx.actor;
        return match lifestyle::pay_arrears(ctx, &cfg, who) {
            Ok(line) => {
                say(ctx, line);
                Ok(())
            }
            Err(e) => Err(refuse_lifestyle(&e)),
        };
    }
    lifestyle_show(ctx, &cfg)
}

fn refuse_lifestyle(e: &LifestyleError) -> CmdError {
    CmdError::msg(e.to_string())
}

fn lifestyle_show(ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) -> Result<(), CmdError> {
    let who = ctx.actor;
    let Some(p) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.profile(who).cloned())
    else {
        return Err(CmdError::msg("You have no nuyen account yet."));
    };
    let cost = lifestyle::tier_amount(ctx, cfg, p.lifestyle);
    let mut lines = vec![format!("Lifestyle: {}", p.lifestyle.label())];
    match (cost, p.account) {
        (Some(c), Some(a)) => lines.push(format!(
            "  cost: {} per {SR_MONTH} period",
            mush_economy::money_on(ctx, a, c)
        )),
        _ => lines.push("  cost: nothing (this tier is free)".to_string()),
    }
    if p.unpaid_periods > 0 {
        let arrears = p
            .account
            .map(|a| mush_economy::money_on(ctx, a, p.arrears))
            .unwrap_or_else(|| p.arrears.minor().to_string());
        lines.push(format!(
            "  ARREARS: {} unpaid period(s), {arrears} owed (+lifestyle/pay).",
            p.unpaid_periods
        ));
    }
    if let Some(o) = lifestyle::upkeep_of(ctx, who) {
        lines.push(format!("  upkeep record: obligation #{}", o.id.0));
    }
    for line in lines {
        say(ctx, line);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// +nuyen
// ---------------------------------------------------------------------------

fn nuyen_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_string();
    let target = if raw.is_empty() {
        ctx.actor
    } else {
        let t = match_object(ctx.w, ctx.actor, &raw)?;
        if t != ctx.actor && !perm::can_examine(ctx.w, ctx.actor, t) {
            return Err(CmdError::msg("You can't examine that character."));
        }
        t
    };
    for line in nuyen_lines(cmd, ctx, target) {
        say(ctx, line);
    }
    Ok(())
}

/// The one line `+nuyen` gives a viewer who may see neither the account nor the
/// sheet — **identical** for a banked character and an unbanked one (D12).
const NO_NUYEN: &str = "Nuyen: nothing to show.";

/// The balance render (§3.5, G2): the account after approval, the sheet field before,
/// and [`NO_NUYEN`] — the *same* line either way — for a viewer the account's own gate
/// refuses.
fn nuyen_lines(cmd: &Sr5Cmd, ctx: &Ctx<'_>, who: ObjId) -> Vec<String> {
    let account = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.profile(who).and_then(|p| p.account));
    if let Some(account) = account
        && may_view_account(ctx.w, ctx.actor, account)
    {
        let balance = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().account(account).map(|a| a.posted_balance))
            .unwrap_or(Amount::ZERO);
        return vec![format!(
            "Nuyen: {} (account #{account})",
            mush_economy::money_on(ctx, account, balance)
        )];
    }
    // D12: an account the viewer may not see must read exactly like no account at
    // all. Returning nothing for the first and a sheet line for the second made
    // `+nuyen <who>`'s **silence** the tell that the character was banked — the
    // existence of the account leaking through the absence of a line. One fallback
    // either way, and it never names an amount.
    if account.is_some() {
        return vec![NO_NUYEN.to_string()];
    }
    match cmd.read(ctx.w, who) {
        // The sheet number is the owner's and staff's (§3.5's opening rule); to
        // anybody else it reads exactly like the unviewable account above.
        Some(doc) if ctx.actor == who || crate::economy::is_staff(ctx.w, ctx.actor) => {
            vec![format!(
                "Nuyen (chargen sheet): {}",
                doc.number("nuyen").unwrap_or(0)
            )]
        }
        Some(_) | None => vec![NO_NUYEN.to_string()],
    }
}

// ---------------------------------------------------------------------------
// @sr
// ---------------------------------------------------------------------------

fn sr_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let cfg = config(ctx);
    let sw = ctx.switches.clone();
    if sw.has("clock") {
        return sr_clock(ctx, &cfg);
    }
    if sw.has("migrate") {
        require_enabled(&cfg)?;
        return sr_migrate(cmd, ctx, &cfg);
    }
    if sw.has("profile") {
        require_enabled(&cfg)?;
        return sr_profile(ctx);
    }
    if sw.has("fence") {
        require_enabled(&cfg)?;
        return sr_fence(ctx, &cfg);
    }
    sr_economy(ctx, &cfg)
}

fn sr_economy(ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) -> Result<(), CmdError> {
    let (profiles, runs, open) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .map(|s| (s.profiles().count(), s.runs().count(), s.open_runs()))
        .unwrap_or((0, 0, 0));
    let instrument = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.instrument.clone())
        .map_or_else(|| "(unregistered)".to_string(), |i| i.to_string());
    let cadence = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(SrEconomyStore::cadence)
        .map_or_else(|| "(missing)".to_string(), |c| format!("#{}", c.0));
    let lines = vec![
        format!("[shadowrun.economy] enabled = {}", cfg.enabled),
        format!(
            "  instrument = {instrument}   lifestyle_clock = {} ({cadence})",
            cfg.lifestyle_clock
        ),
        format!(
            "  lifestyle = {}   player_run_offers = {}   organization_run_offers = {}",
            cfg.lifestyle, cfg.player_run_offers, cfg.organization_run_offers
        ),
        format!(
            "  chargen_shopping = {} (inert until E6c)   chargen_carryover_cap = {} (parsed, not \
             applied)",
            cfg.chargen_shopping, cfg.chargen_carryover_cap
        ),
        format!(
            "  fence rates = {}/{} permille, delay {} s (inert until E6b)",
            cfg.fence_rate_immediate, cfg.fence_rate_consign, cfg.fence_delay_secs
        ),
        format!(
            "  board: {} open of {} cap, {} per principal, ttl {} s",
            open,
            cfg.board_cap,
            cfg.open_runs_cap(),
            cfg.board_ttl()
        ),
        format!("  {profiles} profile(s), {runs} listing(s)"),
    ];
    for line in lines {
        say(ctx, line);
    }
    Ok(())
}

/// `@sr/clock` — **show only** since SP1 (D-LBK-7): `sr.month` is a *declared*
/// cadence, contributed to `[economy.cadences]` by the host and created or adopted by
/// the economy at its own boot. Nothing here creates a clock.
fn sr_clock(ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) -> Result<(), CmdError> {
    let found = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| mush_economy::cadence_by_name(s.snapshot(), &cfg.lifestyle_clock).cloned());
    match found {
        Some(c) => {
            say(
                ctx,
                format!(
                    "{}: cadence #{} on {:?}, period {} s, catch-up {}, position {} (schema {}).",
                    c.name, c.id.0, c.source, c.period, c.catch_up_cap, c.position, c.schema
                ),
            );
            say(
                ctx,
                "Declared in [economy.cadences]; @sr/clock shows it and never creates one."
                    .to_string(),
            );
        }
        None => say(
            ctx,
            format!(
                "{} does not exist. Declare it: [economy.cadences.\"{}\"] clock = \"world_ic\", \
                 period_secs = {}, catch_up = {} — then restart.",
                cfg.lifestyle_clock,
                cfg.lifestyle_clock,
                crate::economy::config::SR_MONTH_PERIOD_SECS,
                crate::economy::config::SR_MONTH_CATCH_UP
            ),
        ),
    }
    Ok(())
}

fn sr_migrate(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @sr/migrate dry|commit [<who>]";
    let raw = ctx.args.lhs().trim().to_string();
    let mut words = raw.split_whitespace();
    let mode = words
        .next()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let who = match words.next() {
        Some(w) => Some(match_object(ctx.w, ctx.actor, w)?),
        None => None,
    };
    let commit = match mode.as_str() {
        "dry" | "" => false,
        "commit" => true,
        _ => return Err(CmdError::msg(USAGE)),
    };
    let config = cmd.config.clone();
    let plan = crate::economy::chargen::plan(ctx, cmd, &config, who);
    if plan.is_empty() {
        say(ctx, "No approved character has anything to migrate.");
        return Ok(());
    }
    say(
        ctx,
        format!(
            "@sr/migrate {} — {} approved character(s):",
            if commit { "commit" } else { "dry" },
            plan.len()
        ),
    );
    for line in plan {
        if !commit {
            let text = match &line.verdict {
                MigrationVerdict::WouldMint(n) => {
                    format!("would mint {n} from system:sr.migration")
                }
                MigrationVerdict::Adopt(n) => {
                    format!("would adopt with the priority allotment ({n})")
                }
                MigrationVerdict::Hit { amount, bound } => {
                    format!(
                        "already migrated ({} at sheet value {bound})",
                        amount.minor()
                    )
                }
                MigrationVerdict::Mismatch { bound } => {
                    format!("MISMATCH — bound {bound}, sheet now {}", line.sheet_value)
                }
                MigrationVerdict::Skip(why) => (*why).to_string(),
            };
            say(ctx, format!("  #{}: {text}", line.who.0));
            continue;
        }
        let stamp = cmd.read(ctx.w, line.who).as_ref().and_then(approval_stamp);
        let (by, at) = stamp.unwrap_or((ctx.actor, ctx.w.now()));
        let report = crate::economy::chargen::commit_one(ctx, cfg, &line, by, at);
        say(ctx, format!("  {report}"));
    }
    Ok(())
}

fn sr_profile(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.lhs().trim().to_string();
    if raw.is_empty() {
        return Err(CmdError::msg("Usage: @sr/profile <who>"));
    }
    let who = match_object(ctx.w, ctx.actor, &raw)?;
    let Some(p) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.profile(who).cloned())
    else {
        return Err(CmdError::msg("That character has no SR economic profile."));
    };
    let mut lines = vec![
        format!("Profile #{}: lifestyle {}", who.0, p.lifestyle.label()),
        format!(
            "  account: {}   upkeep: {}",
            p.account
                .map_or_else(|| "none".to_string(), |a| format!("#{a}")),
            p.upkeep
                .map_or_else(|| "none".to_string(), |o| format!("#{}", o.0))
        ),
        format!(
            "  unpaid periods: {} (since sr.month {})   arrears: {}",
            p.unpaid_periods,
            p.unpaid_since,
            // D10: bare minor units here were the one arrears render in the crate that
            // did not go through the instrument. Staff read the same `¥` everybody
            // else does; `money_on` asks the account's own gate, which staff pass.
            p.account.map_or_else(
                || p.arrears.minor().to_string(),
                |a| mush_economy::money_on(ctx, a, p.arrears)
            )
        ),
        format!(
            "  misses accounted for on the live record: {}",
            p.missed_seen
        ),
    ];
    if let Some(m) = &p.minted {
        lines.push(format!(
            "  minted: {} under {} at {}",
            m.amount.minor(),
            m.key,
            m.at
        ));
    }
    if let Some(m) = &p.migrated {
        lines.push(format!(
            "  migrated: sheet value {} bound, approved by #{} at {}",
            m.from_sheet_value, m.approved_by.0, m.approved_at
        ));
    }
    for line in lines {
        say(ctx, line);
    }
    Ok(())
}

/// The listing states a run may be in, for tests and renders that need the word.
#[must_use]
pub fn state_word(s: RunState) -> &'static str {
    s.word()
}

// ---- E6b: B9 — the `+fence` surface and `@sr/fence` ------------------------

/// `+fence` — the whole player-facing fence surface.
fn fence_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let cfg = config(ctx);
    require_enabled(&cfg)?;
    let sw = ctx.switches.clone();
    if sw.has("show") {
        fence_show(ctx, &cfg)
    } else if sw.has("quote") {
        fence_quote(ctx, &cfg)
    } else if sw.has("accept") {
        fence_accept(ctx, &cfg)
    } else if sw.has("deliver") {
        fence_deliver(ctx)
    } else {
        fence_list(ctx, &cfg)
    }
}

/// `+fence/list` — the registered fences, then this seller's own open quotes.
///
/// A **hidden** fence organization's listings are simply **absent** for a
/// non-member — never "No such" (G3, scenario 12): the economy owns the render
/// (`render_org_listed`), the ruleset owns the absence.
fn fence_list(ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) -> Result<(), CmdError> {
    let fences: Vec<(FenceId, mush_org::OrgId, SmolStr, i64)> = ctx
        .w
        .module_state::<SrEconomyStore>()
        .map(|s| {
            s.fences()
                .map(|f| (f.id, f.org, f.name.clone(), f.demand))
                .collect()
        })
        .unwrap_or_default();
    let mut rows: Vec<String> = Vec::new();
    // E6b verify fold (D3): **filter, then page.** Taking the page before the
    // hidden-org filter made a member's and a non-member's page truncate
    // differently, which is a difference a non-member can read; and the walk is
    // bounded on records **visited**, not on records matched (D-LBK-5's rule).
    for (id, org, name, demand) in fences.into_iter().take(FENCE_PAGE_VISIT) {
        if !mush_economy::may_list_org(ctx, org) {
            continue;
        }
        if rows.len() >= FENCE_PAGE {
            break;
        }
        let immediate = i64::from(cfg.fence_rate_immediate)
            .saturating_add(demand)
            .clamp(0, 1_000);
        let consign = i64::from(cfg.fence_rate_consign)
            .saturating_add(demand)
            .clamp(0, 1_000);
        rows.push(format!(
            "  fence #{id} — {} ({}), immediate {immediate} permille, consign {consign} permille",
            mush_economy::render_org_listed(ctx, org),
            if name.is_empty() {
                "unnamed".to_string()
            } else {
                name.to_string()
            },
        ));
    }
    if rows.is_empty() {
        say(ctx, "No fence is buying here.");
    } else {
        say(ctx, "Fences:".to_string());
        for r in rows {
            say(ctx, r);
        }
    }
    let mine: Vec<String> = quote_lines(ctx, cfg);
    if !mine.is_empty() {
        say(ctx, "Your open quotes:".to_string());
        for r in mine {
            say(ctx, r);
        }
    }
    Ok(())
}

/// One line per open quote of the actor's, newest first, bounded.
fn quote_lines(ctx: &Ctx<'_>, cfg: &SrEconomyConfig) -> Vec<String> {
    let actor = ctx.actor;
    // E6b verify fold (D10): bounded on records **visited**, newest first, so a long
    // quote tail is never an unbounded scan wearing a page cap.
    let quotes: Vec<FenceQuote> = ctx
        .w
        .module_state::<SrEconomyStore>()
        .map(|s| {
            s.quotes()
                .rev()
                .take(FENCE_PAGE_VISIT)
                .filter(|q| q.seller == actor)
                .take(FENCE_PAGE)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    quotes
        .into_iter()
        .map(|q| quote_line(ctx, cfg, &q))
        .collect()
}

fn quote_line(ctx: &Ctx<'_>, cfg: &SrEconomyConfig, q: &FenceQuote) -> String {
    let settlement = match q.settlement {
        FenceSettlement::Immediate => "immediate".to_string(),
        // Answer 15: the delay is **shown in the offer**.
        FenceSettlement::Consign { delay } => format!("consignment in {} h", delay / 3_600),
    };
    let state = if q.contract.is_some() {
        "agreed"
    } else if crate::economy::fence::is_current(ctx, q) {
        "open"
    } else {
        "lapsed"
    };
    format!(
        "  quote #{} — {} for #{} ({}), {settlement}, {state}",
        q.id,
        mush_economy::money(ctx, &instrument_of(cfg), q.offer),
        q.object.0,
        q.catalog
    )
}

fn instrument_of(cfg: &SrEconomyConfig) -> mush_economy::InstrumentId {
    mush_economy::InstrumentId::new(cfg.instrument.as_str())
        .unwrap_or_else(|| mush_economy::InstrumentId::new("nuyen").expect("a well-formed code"))
}

/// An existence-hiding refusal from a `+fence` verb, **counted** (E6b verify fold,
/// D8): `FenceId` and `QuoteId` are two enumerable id spaces, and C16's tally exists
/// so staff can see the shape of a walk over one.
fn no_such(ctx: &mut Ctx<'_>, text: &'static str) -> CmdError {
    mush_economy::note_refusal(ctx, true);
    CmdError::msg(text)
}

/// `+fence/show <quote>` — the whole of the arithmetic, to its seller alone.
fn fence_show(ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) -> Result<(), CmdError> {
    let parsed = parse_u64(ctx.args.lhs()).map(QuoteId);
    let Some(id) = parsed else {
        return Err(no_such(ctx, crate::economy::fence::NO_SUCH_QUOTE));
    };
    let found = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.quote(id).cloned())
        .filter(|q| q.seller == ctx.actor || crate::economy::is_staff(ctx.w, ctx.actor));
    let Some(q) = found else {
        return Err(no_such(ctx, crate::economy::fence::NO_SUCH_QUOTE));
    };
    let line = quote_line(ctx, cfg, &q);
    say(ctx, line);
    // The basis is the whole of the arithmetic: a hidden fact contributes no factor
    // at all, so what the seller reads is what the fence computed (D-E6-11).
    for f in &q.basis {
        say(ctx, format!("    {} {} permille", f.label, f.permille));
    }
    match q.contract {
        Some(c) => say(
            ctx,
            format!(
                "  agreed as contract #{}. Accept it (@contract/accept #{}) and then \
                 +fence/deliver {}.",
                c.0, c.0, q.id
            ),
        ),
        None => say(ctx, format!("  +fence/accept {} takes it up.", q.id)),
    }
    Ok(())
}

/// `+fence/quote <fence>=<obj> [immediate|consign]`.
fn fence_quote(ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: +fence/quote <fence>=<object> [immediate|consign]";
    let parsed = parse_u64(ctx.args.lhs()).map(FenceId);
    let Some(fence) = parsed else {
        return Err(no_such(ctx, crate::economy::fence::NO_SUCH_FENCE));
    };
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let mut words = rhs.split_whitespace();
    let target = words
        .next()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .to_string();
    let consign = match words.next().map(str::to_ascii_lowercase).as_deref() {
        None | Some("immediate") => false,
        Some("consign" | "consignment") => true,
        Some(_) => return Err(CmdError::msg(USAGE)),
    };
    let resolved = match_object(ctx.w, ctx.actor, &target).ok();
    let Some(object) = resolved else {
        return Err(no_such(ctx, crate::economy::fence::NO_SUCH_ASSET));
    };
    match crate::economy::fence::quote(ctx, cfg, fence, object, consign) {
        Ok(id) => {
            let q = ctx
                .w
                .module_state::<SrEconomyStore>()
                .and_then(|s| s.quote(id).cloned());
            if let Some(q) = q {
                let line = quote_line(ctx, cfg, &q);
                say(ctx, line);
            }
            say(ctx, format!("  +fence/accept {id} takes it up."));
            Ok(())
        }
        Err(e) => Err(crate::economy::fence::refuse_fence(
            ctx,
            &e,
            &[NamedResource::Asset(object)],
        )),
    }
}

/// `+fence/accept <quote>` — draft, offer, and let the fence escrow first.
fn fence_accept(ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) -> Result<(), CmdError> {
    let parsed = parse_u64(ctx.args.lhs()).map(QuoteId);
    let Some(id) = parsed else {
        return Err(no_such(ctx, crate::economy::fence::NO_SUCH_QUOTE));
    };
    match crate::economy::fence::accept(ctx, cfg, id) {
        Ok(contract) => {
            say(
                ctx,
                format!(
                    "The fence has escrowed its payment as contract #{}. Accept it \
                     (@contract/accept #{}) and only then +fence/deliver {id} — the goods stay \
                     with you until the money is committed.",
                    contract.0, contract.0
                ),
            );
            Ok(())
        }
        Err(e) => Err(crate::economy::fence::refuse_fence(ctx, &e, &[])),
    }
}

/// `+fence/deliver <quote>`.
fn fence_deliver(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let parsed = parse_u64(ctx.args.lhs()).map(QuoteId);
    let Some(id) = parsed else {
        return Err(no_such(ctx, crate::economy::fence::NO_SUCH_QUOTE));
    };
    let object = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.quote(id).map(|q| q.object));
    match crate::economy::fence::deliver(ctx, id) {
        Ok(line) => {
            say(ctx, line);
            Ok(())
        }
        Err(e) => {
            let named: Vec<NamedResource> = object.map(NamedResource::Asset).into_iter().collect();
            Err(crate::economy::fence::refuse_fence(ctx, &e, &named))
        }
    }
}

/// `@sr/fence <org>=<vault> [name=<text>] [demand=<permille>]` (staff).
fn sr_fence(ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) -> Result<(), CmdError> {
    const USAGE: &str =
        "Usage: @sr/fence <org>=<vault> [name=<text>] [demand=<permille, -1000..1000>]";
    let org = mush_economy::resolve_org_visible(ctx, ctx.args.lhs())
        .ok_or_else(|| CmdError::msg("No such organization."))?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?;
    let (rest, opts) = parse_opts(rhs, &["name", "demand"]);
    let vault = parse_u64(&rest)
        .map(mush_economy::VaultId)
        .ok_or_else(|| CmdError::msg("No such vault."))?;
    let name = SmolStr::new(opts.get("name").map(String::as_str).unwrap_or("").trim());
    let demand = match opts.get("demand") {
        Some(s) => s
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|n| (-1_000..=1_000).contains(n))
            .ok_or_else(|| CmdError::msg(USAGE))?,
        None => 0,
    };
    // DL-11 is **composition data**: an org the wiring never named cannot have its
    // acceptance performed programmatically, so registering it as a fence would mint
    // a shop nobody could ever sell to. Refuse with the exact fix (08a's
    // staff-scarce rule: never park state on "ask someone", say the command).
    if !cfg.is_vendor_org(org) {
        return Err(CmdError::msg(format!(
            "org #{} is not a wiring-named vendor. Add it to [shadowrun.economy].vendor_orgs = \
             [{}] in mushd.toml and restart; the list is composition data on purpose — it is what \
             lets a fence accept a deal with no officer connected.",
            org.0, org.0
        )));
    }
    match crate::economy::fence::register_fence(ctx, cfg, org, vault, name, demand) {
        Ok(id) => {
            say(ctx, format!("Fence #{id} registered."));
            Ok(())
        }
        Err(e) => Err(crate::economy::fence::refuse_fence(ctx, &e, &[])),
    }
}
