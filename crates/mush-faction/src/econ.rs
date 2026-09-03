//! E3d — the faction → backing-organization link, and the one-way migration of a
//! [`Faction::resources`](crate::Faction::resources) key into real money
//! (design: `docs/design/08b-e3-implementation-plan.md` §6, D-E3-8 / D-E3-9).
//!
//! ## Two separations this cut enforces
//!
//! - **A faction is not a principal.** The registry says who belongs to the
//!   Camarilla; it has never owned anything. Value attributed to "the Camarilla"
//!   is owned by an [`OrgId`] — a real principal with scopes, grants, an audit
//!   trail, and accounts in the double-entry journal. The faction merely *names*
//!   one, through [`Faction::org`](crate::Faction::org).
//! - **A rating is not money.** `Faction.resources` is saturating, unaudited,
//!   ruleset-declared flavor ("influence.police = 3"). Money is journal-authored,
//!   conserved, and reversible. The two never silently swap places: a key becomes
//!   money only by an explicit, confirmed staff act, recorded per key in
//!   [`Faction::migrated`](crate::Faction::migrated) (D-E3-9). A *dynamic*
//!   classifier — "it is money iff an instrument by that code is registered" —
//!   was rejected: `@instrument/register` would then retroactively make every
//!   faction's like-named rating mintable.
//!
//! ## The link is validated at every use, never at boot
//!
//! `FactionModule` registers **before** `OrgModule` (`mush-server/src/modules.rs`),
//! so at faction boot there is no org registry to ask, and E3 does not reorder the
//! list. [`backing_org`] is therefore the single resolver every consumer calls,
//! and it fails closed three ways: unbound, the org gone, or **two factions naming
//! the same org** — in which case *both* links are unusable until staff re-bind,
//! because there is no safe way to guess which faction the money belongs to.
//!
//! ## Migration ordering: post first, then mark
//!
//! [`faction_migrate`] resolves everything — instrument, org, root scope,
//! destination account, source account, amount — and only then calls
//! [`mush_economy::post`], which is the LAST fallible call it makes (§4.2 step 6).
//! The faction doc is rewritten *after* a successful post. That order converges:
//! if the faction-doc write drops, the world reboots with the money posted and the
//! key still present, and re-running the identical command is an idempotency HIT
//! (`faction.migrate/{fid}/{code}`) that moves nothing and just re-marks. The
//! reverse order would lose the pool with nothing minted to replace it.
//!
//! Two consequences of that ordering are load-bearing enough to name:
//!
//! - **Opening the destination wallet is a store mutation, so every refusal that
//!   can be decided up front is decided ABOVE it.** `open_root_account` advances the
//!   account allocator and puts a record in RAM under a *deferred* persist; a handler
//!   that then returns `Err` loses the persist but keeps the RAM record, leaving a
//!   phantom account that binds the org's root scope until the next reboot. So the
//!   whole-economy freeze, the backing org's ability to *receive* (§4.4), the
//!   instrument, the source account, and the pool are all proved before the wallet is
//!   opened — and if `post` refuses anyway, the refusal is rendered to the actor and
//!   the command returns `Ok` so the wallet's persist rides out with it (an org
//!   root-scope account is a legitimate record whether or not the mint landed).
//! - **The idempotency index, not the faction doc, answers "did this migrate?"**
//!   A re-run rebuilds the plan from the *current* pool, so a pool staff edited
//!   between the post and the re-run hashes differently under the same key and
//!   `post` refuses with `IdempotencyConflict` — forever, with the money in the
//!   wallet AND the rating still live. [`faction_migrate`] therefore asks the index
//!   before it looks at the pool, and converges on a hit instead of re-planning.

use mush_core::{CmdError, Ctx, ObjId, World};
use mush_economy::{
    Account, AccountId, AccountStatus, Amount, CreditPolicy, EconomicCause, EconomyPlan,
    EconomyStore, IdempotencyKey, InstrumentId, InstrumentStatus, PostOutcome, RequestedLine,
    TransactionId,
};
use mush_org::{
    OrgAction, OrgId, OrgMutationOp, OrgMutationPlan, OrgScope, OrgScopeId, OrgStatus, OrgStore,
    Organization, PrincipalId, RulesValue, ScopeStatus,
};
use smol_str::SmolStr;

use crate::data::{Faction, FactionId, FactionState, ResourceChange, persist_faction};

// ----------------------------------------------------------------------
// The resolver
// ----------------------------------------------------------------------

/// Why a faction's backing-organization link could not be resolved. Every variant
/// is a **fail-closed** answer: no money command proceeds on one.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// No organization is bound (the default for every faction).
    Unbound,
    /// The bound organization is not in the org registry — destroyed, never
    /// restored, or bound before it existed.
    Missing(OrgId),
    /// Another faction names the same organization. **Both** links are unusable:
    /// picking one would attribute money to whichever faction happened to be
    /// scanned first.
    Conflict(OrgId),
}

impl LinkError {
    /// The `@faction/show` label for this outcome (`org #3 (MISSING)`,
    /// `no backing org`, …).
    #[must_use]
    pub fn label(self) -> String {
        match self {
            LinkError::Unbound => "no backing org".to_string(),
            LinkError::Missing(org) => format!("org #{org} (MISSING)"),
            LinkError::Conflict(org) => format!("org #{org} (CONFLICT)"),
        }
    }
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::Unbound => f.write_str(
                "That faction has no backing organization. \
                 (@faction/org <faction>=new  mints one.)",
            ),
            LinkError::Missing(org) => write!(
                f,
                "That faction names org #{org}, which no longer exists; re-bind it first."
            ),
            LinkError::Conflict(org) => write!(
                f,
                "org #{org} backs more than one faction, so neither link is usable \
                 until staff re-bind them."
            ),
        }
    }
}

/// **The** faction → organization resolver (D-E3-8). Every consumer — the show
/// renderers, the migration, the destroy gate's message — goes through here, so
/// there is exactly one definition of "this faction's money identity".
///
/// Checked live, in order: bound at all, the org still exists, and no *other*
/// faction claims it. The org's `status` is deliberately NOT checked here: this
/// answers "which organization", and whether a suspended org may be credited or
/// debited is the economy's question, answered by its own live gates.
///
/// # Errors
/// [`LinkError`] — unbound, missing, or claimed by two factions.
pub fn backing_org(w: &World, f: &Faction) -> Result<OrgId, LinkError> {
    let Some(org) = f.org else {
        return Err(LinkError::Unbound);
    };
    let known = w
        .module_state::<OrgStore>()
        .is_some_and(|s| s.state().org(org).is_some());
    if !known {
        return Err(LinkError::Missing(org));
    }
    // The live duplicate scan (F6). Binding checks this too, but a check at bind
    // time alone would be defeated by a restored backup, a hand-edited doc, or two
    // binds racing a reboot — and the failure mode is money attributed to the wrong
    // faction, so it is re-proved at every use.
    if other_holder(w, org, f.id).is_some() {
        return Err(LinkError::Conflict(org));
    }
    Ok(org)
}

/// Boot-time link validation (owner decision 2026-09-03). `OrgModule` now loads
/// before `FactionModule`, so every `Faction.org` link is checked once against the
/// loaded org state. Findings are **reported, never repaired**: a money linkage is
/// not silently dropped (doc 08 §15.2), and [`backing_org`] remains the at-use
/// enforcement — a `MISSING` or `CONFLICT` link fails closed until staff re-bind.
/// With no org module registered (a test assembly) there is nothing to check
/// against, so the links are left as they are with one warning.
pub(crate) fn validate_links_at_boot(w: &World) {
    let Some(st) = w.module_state::<FactionState>() else {
        return;
    };
    let bound = st.factions.values().filter(|f| f.org.is_some()).count();
    if bound == 0 {
        return;
    }
    if w.module_state::<OrgStore>().is_none() {
        tracing::warn!(
            bound,
            "faction: no org module registered; backing-org links cannot be validated at boot"
        );
        return;
    }
    for f in st.factions.values() {
        match backing_org(w, f) {
            Ok(_) | Err(LinkError::Unbound) => {}
            Err(LinkError::Missing(org)) => tracing::error!(
                faction = f.id.0,
                %org,
                "faction: backing org is missing; the link fails closed until staff re-bind it"
            ),
            Err(LinkError::Conflict(org)) => tracing::error!(
                faction = f.id.0,
                %org,
                other = ?other_holder(w, org, f.id),
                "faction: backing org is claimed by another faction; both links fail closed until staff re-bind"
            ),
        }
    }
}

/// The name of a faction OTHER than `except` that binds `org`, if any.
fn other_holder(w: &World, org: OrgId, except: FactionId) -> Option<SmolStr> {
    let st = w.module_state::<FactionState>()?;
    st.factions
        .values()
        .find(|f| f.id != except && f.org == Some(org))
        .map(|f| f.name.clone())
}

/// The `@faction/show` link line: `org #3`, `org #3 (MISSING)`,
/// `org #3 (CONFLICT)`, or `no backing org`.
pub(crate) fn link_label(w: &World, f: &Faction) -> String {
    match backing_org(w, f) {
        Ok(org) => format!("org #{org}"),
        Err(e) => e.label(),
    }
}

// ----------------------------------------------------------------------
// The view-gated money read-through (§4.5, D-E3-6)
// ----------------------------------------------------------------------

/// The backing org's root-scope account balances, rendered ONLY to viewers who may
/// see each account: staff, or a holder of `economy.account.view` on that scope.
///
/// The gate is [`mush_economy::may_view_account`] — the economy's own §4.5 helper,
/// not a second implementation — and a viewer without standing gets **nothing**,
/// not a "hidden" marker: the faction officer gate governs *ratings*, and reusing
/// it for money would hand every officer a read of the org's books through a back
/// door the economy never granted (D-E3-11: a faction role is not an
/// `OrgMembership`).
pub(crate) fn balance_lines(ctx: &Ctx<'_>, f: &Faction, indent: &str) -> Vec<String> {
    let Ok(org) = backing_org(ctx.w, f) else {
        return Vec::new();
    };
    let Some(root) = ctx
        .w
        .module_state::<OrgStore>()
        .and_then(|s| s.state().org(org).map(|o| o.root_scope))
    else {
        return Vec::new();
    };
    let Some(store) = ctx.w.module_state::<EconomyStore>() else {
        return Vec::new();
    };
    let snap = store.snapshot();
    let owner = PrincipalId::Organization(org);
    snap.accounts()
        .filter(|a| a.scope == Some(root) && a.owner == owner)
        .filter(|a| mush_economy::may_view_account(ctx.w, ctx.actor, a.id))
        .map(|a| {
            let scale = snap.instrument(&a.instrument).map_or(0, |i| i.scale);
            format!(
                "{indent}Account #{} {} = {}",
                a.id,
                a.instrument,
                a.posted_balance.to_decimal_string(scale)
            )
        })
        .collect()
}

// ----------------------------------------------------------------------
// Shared helpers
// ----------------------------------------------------------------------

fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "faction", msg.into());
}

/// Set (or clear) a resolved faction's backing org, returning the updated doc.
fn set_org(ctx: &mut Ctx<'_>, id: FactionId, org: Option<OrgId>) -> Faction {
    let st = ctx
        .w
        .module_state_mut::<FactionState>()
        .expect("faction state slot installed at registration");
    let fac = st.factions.get_mut(&id).expect("resolved above");
    fac.org = org;
    fac.clone()
}

/// Resolve an organization by `#id` / `id` or by case-insensitive name/alias —
/// the `@org` command's own resolution rule, kept here so the faction crate never
/// reaches into org internals.
fn resolve_org(w: &World, s: &str) -> Option<OrgId> {
    let s = s.trim();
    let store = w.module_state::<OrgStore>()?;
    if let Ok(n) = s.trim_start_matches('#').parse::<u64>() {
        return store.state().org(OrgId(n)).map(|o| o.id);
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

fn parse_account(s: &str) -> Option<AccountId> {
    s.trim().trim_start_matches('#').parse::<u64>().ok().map(AccountId)
}

// ----------------------------------------------------------------------
// @faction/org — bind, mint-and-bind, unbind
// ----------------------------------------------------------------------

/// `@faction/org <faction>=<org|new|none>` (staff).
///
/// Money attribution is a deliberate act, so `@faction/create` never auto-binds
/// and this is the only way in or out. `none` unbinds and leaves the organization,
/// its scopes, and every account it owns exactly where they are — unbinding is
/// "this faction no longer speaks for that org", never a deletion.
pub(crate) fn faction_org(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "@faction/org <faction>=<org|new|none>";
    let f = crate::commands::resolve_faction(ctx, ctx.args.lhs(), false)?;
    let rhs = crate::commands::rhs_required(ctx, USAGE)?;
    match rhs.trim().to_ascii_lowercase().as_str() {
        "none" => org_unbind(ctx, &f),
        "new" => org_mint_and_bind(ctx, &f),
        _ => org_bind(ctx, &f, rhs.trim()),
    }
}

fn org_unbind(ctx: &mut Ctx<'_>, f: &Faction) -> Result<(), CmdError> {
    let Some(org) = f.org else {
        return Err(CmdError::msg(format!(
            "{} has no backing organization.",
            f.name
        )));
    };
    let updated = set_org(ctx, f.id, None);
    persist_faction(ctx.out, &updated);
    info(
        ctx,
        format!(
            "{} is no longer backed by org #{org}. The organization and its \
             accounts are untouched.",
            updated.name
        ),
    );
    Ok(())
}

fn org_bind(ctx: &mut Ctx<'_>, f: &Faction, spec: &str) -> Result<(), CmdError> {
    let org = resolve_org(ctx.w, spec).ok_or_else(|| CmdError::msg("No such organization."))?;
    if f.org == Some(org) {
        return Err(CmdError::msg(format!(
            "{} is already backed by org #{org}.",
            f.name
        )));
    }
    // Only a live organization may be bound: binding a Suspended/Dissolving/Closed
    // one would create a faction whose every money command refuses at the economy's
    // gate, with an error that says nothing about the real cause.
    let active = ctx
        .w
        .module_state::<OrgStore>()
        .and_then(|s| s.state().org(org).map(|o| o.status == OrgStatus::Active))
        .unwrap_or(false);
    if !active {
        return Err(CmdError::msg(format!(
            "org #{org} is not active; only an active organization may back a faction."
        )));
    }
    // One org backs at most one faction. Checked here so staff get a real message,
    // and re-checked live by `backing_org` so a duplicate that arrives any other
    // way still fails closed.
    if let Some(other) = other_holder(ctx.w, org, f.id) {
        return Err(CmdError::msg(format!(
            "org #{org} already backs faction {other}; unbind it there first."
        )));
    }
    if let Some(old) = f.org {
        info(ctx, format!("(was org #{old}; it is left standing.)"));
    }
    let updated = set_org(ctx, f.id, Some(org));
    persist_faction(ctx.out, &updated);
    info(
        ctx,
        format!("{} is now backed by org #{org}.", updated.name),
    );
    Ok(())
}

fn org_mint_and_bind(ctx: &mut Ctx<'_>, f: &Faction) -> Result<(), CmdError> {
    if let Some(org) = f.org {
        return Err(CmdError::msg(format!(
            "{} is already backed by org #{org}; unbind it first \
             (@faction/org {}=none).",
            f.name, f.name
        )));
    }
    if ctx.w.module_state::<OrgStore>().is_none() {
        return Err(CmdError::msg("Organizations are not available on this world."));
    }
    let name = f.name.to_string();
    // Exactly `@org/create`'s shape: allocate the org + its root scope from the
    // slot's monotonic allocators, then create both in ONE `CreateOrg` op so the
    // org and the scope every account will hang off commit together.
    let (org_id, root_id) = {
        let store = ctx.w.module_state_mut::<OrgStore>().expect("checked above");
        (store.alloc_org(), store.alloc_scope())
    };
    let org = Organization {
        id: org_id,
        name: name.clone().into(),
        aliases: Vec::new(),
        status: OrgStatus::Active,
        hidden: false,
        charter: String::new(),
        root_scope: root_id,
        revision: 1,
        ruleset_ext: RulesValue::default(),
    };
    let root = OrgScope {
        id: root_id,
        org: org_id,
        parent: None,
        name: "root".into(),
        status: ScopeStatus::Active,
        depth: 0,
        created_rev: 1,
        revision: 1,
    };
    let plan = OrgMutationPlan::of(vec![OrgMutationOp::CreateOrg { org, root }]);
    // `submit` is terminal on its success path and is the last FALLIBLE call here:
    // everything below only mutates faction state and queues writes. A dropped
    // faction-doc write therefore leaves an unbound org standing (staff re-bind it),
    // never a faction pointing at an org that was never created.
    mush_org::submit(
        ctx,
        &plan,
        OrgAction {
            action: SmolStr::new_static("org.create"),
            org: Some(org_id),
            detail: serde_json::json!({ "name": name, "faction": f.id.0 }),
        },
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;
    let updated = set_org(ctx, f.id, Some(org_id));
    persist_faction(ctx.out, &updated);
    info(
        ctx,
        format!(
            "Created org #{org_id} ({name}); root scope #{root_id}. \
             {} is now backed by org #{org_id}.",
            updated.name
        ),
    );
    Ok(())
}

// ----------------------------------------------------------------------
// @faction/migrate — one rating key becomes money, once
// ----------------------------------------------------------------------

/// `@faction/migrate <faction>=<key> <sysacct> [/confirm]` (staff, ONE key per
/// call).
///
/// The scale question (F2) has one answer: the rating integer is taken **verbatim
/// as minor units**. A pool of `250` on a scale-2 instrument becomes `2.50`, and
/// the un-confirmed run prints exactly that so staff see the interpretation before
/// anything mints. There is no guessing and no multiplier.
pub(crate) fn faction_migrate(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "@faction/migrate <faction>=<kind> <system account> [/confirm]";
    let f = crate::commands::resolve_faction(ctx, ctx.args.lhs(), false)?;
    let rhs = crate::commands::rhs_required(ctx, USAGE)?;
    let mut words = rhs.split_whitespace();
    let (Some(raw_key), Some(raw_src), None) = (words.next(), words.next(), words.next()) else {
        return Err(CmdError::msg(format!("Usage: {USAGE}")));
    };
    {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return Err(CmdError::msg("The economy is not available on this world."));
        };
        // 0. The whole-economy freeze, hoisted to the top of the handler rather than
        //    left to `post`. `post` would refuse it just the same — but only AFTER
        //    `open_root_account` has already advanced the allocator and put a wallet
        //    in RAM, and a frozen economy is precisely the state in which a phantom
        //    account must not be minted.
        if store.is_frozen() {
            return Err(CmdError::msg(
                "The economy is frozen; nothing may be posted or opened until staff clear it.",
            ));
        }
    }

    // 1. The key must canonicalize to a REGISTERED, ACTIVE instrument code (F4).
    //    A suspended or retired instrument would refuse the post anyway; refusing
    //    here keeps the marking and the money in step.
    let code = InstrumentId::new(raw_key).ok_or_else(|| {
        CmdError::msg(format!(
            "{raw_key:?} is not a usable instrument code ([a-z0-9_.-]), so it cannot be money."
        ))
    })?;
    let key = SmolStr::new(code.as_str());
    let scale = {
        let store = ctx.w.module_state::<EconomyStore>().expect("checked above");
        let def = store.snapshot().instrument(&code).ok_or_else(|| {
            CmdError::msg(format!(
                "No instrument {code} is registered; register it before migrating a pool into it."
            ))
        })?;
        if def.status != InstrumentStatus::Active {
            return Err(CmdError::msg(format!(
                "Instrument {code} is {:?}, not Active; nothing may be posted into it.",
                def.status
            )));
        }
        def.scale
    };

    // 2. The faction must resolve a backing org, live — and that org must be able to
    //    RECEIVE. `backing_org` deliberately answers "which organization" and leaves
    //    the lifecycle question to the economy, but the economy answers it inside
    //    `post` (`credit_allowed`, §4.4: a `Dissolving` org takes final settlement, a
    //    `Suspended` or `Closed` one takes nothing), which is one call too late — the
    //    wallet would already be open. So the same rule is proved here, first.
    let org = backing_org(ctx.w, &f).map_err(|e| CmdError::msg(e.to_string()))?;
    let (root, org_status) = ctx
        .w
        .module_state::<OrgStore>()
        .and_then(|s| s.state().org(org).map(|o| (o.root_scope, o.status)))
        .ok_or_else(|| CmdError::msg(format!("org #{org} has no root scope.")))?;
    if !matches!(org_status, OrgStatus::Active | OrgStatus::Dissolving) {
        return Err(CmdError::msg(format!(
            "org #{org} is {org_status:?}; a suspended or closed organization may not \
             receive value, so there is nothing to migrate into."
        )));
    }

    // 3. The idempotency key is a pure function of (faction, instrument), so every
    //    re-run of this migration — including one after a lost faction-doc write —
    //    resolves to the SAME transaction instead of minting a second time.
    let idempotency = IdempotencyKey::new(
        SmolStr::new_static("faction.migrate"),
        SmolStr::new(format!("{}/{code}", f.id)),
    );
    let pool = f.resources.get(&key).copied().unwrap_or(0);

    // 3a. **Did this migration already post?** The economy's index is asked BEFORE
    //     the pool is judged, because the pool is exactly what a re-run cannot trust:
    //     a post that landed with the faction-doc write lost leaves the rating live,
    //     and staff who then edit it (`@faction/resource`) change the plan this
    //     command would rebuild — same idempotency key, different plan hash, so
    //     `post` refuses `IdempotencyConflict` on that run and on every run after it,
    //     with the money in the wallet AND the rating still standing. Editing the
    //     pool to 0 instead dies on "holds no pool". Neither ever marks the key. On a
    //     hit there is nothing left to post, so converge on the doc instead. (This
    //     subsumes the old `pool == 0 && migrated` report.)
    let posted = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().idempotency(&idempotency).map(|r| r.transaction));
    if let Some(tx) = posted {
        return converge_already_posted(ctx, &f, &key, tx);
    }
    // Marked with the pool gone but NO transaction under the key: the marking and the
    // journal disagree in the one direction this command cannot repair (it would have
    // to mint an amount nobody recorded).
    if pool == 0 && f.migrated.contains(&key) {
        return Err(CmdError::msg(format!(
            "{}: {key} is already marked migrated but no migration transaction \
             exists for it; correct it through @econ, not here.",
            f.name
        )));
    }
    // 4. Nothing to mint / no minting debt.
    if pool <= 0 {
        return Err(CmdError::msg(if pool == 0 {
            format!("{} holds no {key} pool; there is nothing to migrate.", f.name)
        } else {
            format!(
                "{} holds a NEGATIVE {key} pool ({pool}). Minting debt is not a thing \
                 the journal can express — settle it with @faction/resource first.",
                f.name
            )
        }));
    }
    let amount = Amount::from_minor(pool);
    let neg = amount
        .checked_neg()
        .ok_or_else(|| CmdError::msg("That pool is out of range for an amount."))?;

    // 5. The source must be a registered system source/sink of the same instrument:
    //    a migration MINTS (the rating had no counterparty), and minting is what a
    //    faucet is for. The economy re-checks that a system debit is staff (A6).
    let source = parse_account(raw_src)
        .ok_or_else(|| CmdError::msg(format!("Usage: {USAGE}")))?;
    {
        let store = ctx.w.module_state::<EconomyStore>().expect("checked above");
        let src = store
            .snapshot()
            .account(source)
            .ok_or_else(|| CmdError::msg("No such account."))?;
        if !matches!(src.owner, PrincipalId::System(_)) {
            return Err(CmdError::msg(format!(
                "Account #{source} is not a system source/sink. A migration mints \
                 value that never had a counterparty, so it must come from one \
                 (@account/system <name>={code})."
            )));
        }
        if src.instrument != code {
            return Err(CmdError::msg(format!(
                "Account #{source} holds {}, not {code}.",
                src.instrument
            )));
        }
        // Hoisted for the same reason as the freeze and the org status: `post`
        // refuses a non-`Active` line account (`InactiveAccount`), and by then the
        // destination wallet exists.
        if src.status != AccountStatus::Active {
            return Err(CmdError::msg(format!(
                "Account #{source} is {:?}, not Active; nothing may be posted from it.",
                src.status
            )));
        }
    }

    // 6. The destination: the org's root-scope account for this instrument.
    let existing = root_account(ctx.w, org, root, &code);
    if !ctx.switches.has("confirm") {
        let target = existing.map_or_else(
            || format!("a NEW org account on root scope #{root}"),
            |id| format!("account #{id}"),
        );
        info(
            ctx,
            format!(
                "{}: {key} = {pool} would move VERBATIM as {} {code} into {target}, \
                 minted from account #{source}.",
                f.name,
                amount.to_decimal_string(scale)
            ),
        );
        info(
            ctx,
            format!(
                "Nothing has moved. Re-run with /confirm to post it: \
                 @faction/migrate/confirm {}={key} #{source}",
                f.name
            ),
        );
        return Ok(());
    }
    // THE store mutation of this handler. Everything that could refuse the post on
    // facts already in hand has refused above; what remains below cannot be answered
    // without asking `post` itself.
    let (dest, opened) = match existing {
        Some(id) => (id, false),
        None => (open_root_account(ctx, org, root, &code), true),
    };

    let mut plan = EconomyPlan::new(
        idempotency,
        PrincipalId::Object(ctx.actor),
        EconomicCause::Staff { reason: "faction resource migration".to_string() },
        vec![
            RequestedLine { account: source, delta: neg, memo: SmolStr::default() },
            RequestedLine { account: dest, delta: amount, memo: SmolStr::default() },
        ],
    );
    plan.description = format!("faction {} resource {code} migration", f.name);
    // §4.2 step 6 — `post` is the LAST fallible call. Everything above is resolved;
    // everything below is infallible bookkeeping over an outcome that has already
    // committed (or already had committed, on a HIT).
    let outcome = mush_economy::post(ctx, &plan);
    let tx = match outcome {
        Ok(PostOutcome::Posted(tx) | PostOutcome::AlreadyPosted(tx)) => tx,
        Ok(PostOutcome::NeedsApproval(id)) => {
            info(
                ctx,
                format!(
                    "Approval required: request #{id}. Nothing has moved, and {key} \
                     is still a rating."
                ),
            );
            return Ok(());
        }
        // A refusal we could not hoist (an unforeseen policy on the source, an
        // arithmetic edge). If a wallet was opened above, `Err` is the ONE answer
        // that must not be given: core discards a failed handler's deferred persists
        // but not its world mutations, so the account would live in RAM, hold the
        // allocator's advance, and bind the root scope in `EconomyScopeUse` with no
        // doc behind it until the next reboot. Report the refusal and return `Ok` so
        // the empty wallet persists as the ordinary record it is.
        Err(e) if opened => {
            let actor = ctx.actor;
            ctx.out.error(
                actor,
                format!(
                    "{}: {key} was NOT migrated — the post was refused ({e}). Nothing \
                     moved and {key} is still a rating; the org's now-empty root-scope \
                     account #{dest} stands (close it with @account/close if it is not \
                     wanted).",
                    f.name
                ),
            );
            return Ok(());
        }
        Err(e) => return Err(CmdError::msg(e.to_string())),
    };

    // Post-then-mark. The `ResourceChange` goes STRAIGHT into the log:
    // `apply_resource` would re-create the key it just removed, leaving a zeroed
    // rating shadowing money that now lives in the journal (F3).
    let now = ctx.w.now();
    let actor: ObjId = ctx.actor;
    let updated = {
        let st = ctx
            .w
            .module_state_mut::<FactionState>()
            .expect("faction state slot installed at registration");
        let fac = st.factions.get_mut(&f.id).expect("resolved above");
        let removed = fac.resources.remove(&key).unwrap_or(0);
        fac.migrated.insert(key.clone());
        fac.push_resource_log(ResourceChange {
            kind: key.clone(),
            delta: -removed,
            reason: format!("migrated to account #{dest}"),
            by: actor,
            ts: now,
        });
        fac.clone()
    };
    persist_faction(ctx.out, &updated);
    info(
        ctx,
        format!(
            "Migrated {}: {key} {} -> account #{dest} (transaction #{tx}). {key} is \
             money now — move it with @econ/pay, mint more with @econ/mint.",
            updated.name,
            amount.to_decimal_string(scale)
        ),
    );
    Ok(())
}

/// The migration under this key ALREADY posted — the economy's idempotency index
/// says so — and the faction doc has not caught up. Converge on the doc: drop
/// whatever the rating still holds, mark the key, log the residual, and report the
/// transaction that did the work.
///
/// This is the only path that can retire a key whose pool moved between the post and
/// the re-run, and it is deliberately **not** gated on `/confirm`: it mints nothing,
/// moves nothing, and describes a world where the money already exists. The residual
/// is *dropped, not minted* — the post that happened is the whole migration, and a
/// rating edited after it was never money (D-E3-9's one-way rule cuts both ways).
///
/// Idempotent in its own right: a second call finds nothing to remove and nothing to
/// mark, writes no doc, and appends no log line.
fn converge_already_posted(
    ctx: &mut Ctx<'_>,
    f: &Faction,
    key: &SmolStr,
    tx: TransactionId,
) -> Result<(), CmdError> {
    let now = ctx.w.now();
    let actor: ObjId = ctx.actor;
    let (residual, updated) = {
        let st = ctx
            .w
            .module_state_mut::<FactionState>()
            .expect("faction state slot installed at registration");
        let fac = st.factions.get_mut(&f.id).expect("resolved above");
        let removed = fac.resources.remove(key);
        let newly = fac.migrated.insert(key.clone());
        let residual = removed.unwrap_or(0);
        if residual != 0 {
            fac.push_resource_log(ResourceChange {
                kind: key.clone(),
                delta: -residual,
                reason: format!("migration already posted as #{tx}; residual dropped"),
                by: actor,
                ts: now,
            });
        }
        // No write when nothing changed: the common re-run is a pure report.
        let changed = removed.is_some() || newly;
        (residual, changed.then(|| fac.clone()))
    };
    if let Some(updated) = &updated {
        persist_faction(ctx.out, updated);
    }
    let msg = if residual == 0 {
        format!("{}: {key} was already migrated (transaction #{tx}). Nothing moved.", f.name)
    } else {
        format!(
            "{}: {key} was already migrated (transaction #{tx}); the {residual} left on the \
             rating was DROPPED, not minted — that post was the migration, and a rating \
             edited after it never became money. Mint more with @econ/mint if it should \
             have been.",
            f.name
        )
    };
    info(ctx, msg);
    Ok(())
}

/// The org's root-scope account for `code`, if one is already open. Preference is
/// for an `Active` one; a closed/frozen account is still returned rather than
/// silently opening a second wallet beside it (the post then refuses loudly).
fn root_account(w: &World, org: OrgId, root: OrgScopeId, code: &InstrumentId) -> Option<AccountId> {
    let store = w.module_state::<EconomyStore>()?;
    let owner = PrincipalId::Organization(org);
    let mut fallback: Option<AccountId> = None;
    for a in store.snapshot().accounts() {
        if a.scope != Some(root) || a.owner != owner || a.instrument != *code {
            continue;
        }
        if a.status == AccountStatus::Active {
            return Some(a.id);
        }
        fallback = fallback.or(Some(a.id));
    }
    fallback
}

/// Open the org's root-scope wallet for `code` — the same record `@account/open
/// org:<org>=<instrument>` builds: credit `forbid()`, because an org wallet is not
/// a faucet and must never be the thing that quietly runs negative.
fn open_root_account(
    ctx: &mut Ctx<'_>,
    org: OrgId,
    root: OrgScopeId,
    code: &InstrumentId,
) -> AccountId {
    let label = {
        let store = ctx.w.module_state::<OrgStore>().expect("resolved above");
        let state = store.state();
        let oname = state.org(org).map_or_else(|| format!("#{org}"), |o| o.name.to_string());
        let sname = state.scope(root).map_or_else(|| "root".to_string(), |s| s.name.to_string());
        SmolStr::new(format!("{oname}/{sname}"))
    };
    let id = ctx
        .w
        .module_state_mut::<EconomyStore>()
        .expect("checked above")
        .alloc_account();
    mush_economy::open_account(
        ctx,
        Account {
            id,
            owner: PrincipalId::Organization(org),
            instrument: code.clone(),
            scope: Some(root),
            name: label,
            status: AccountStatus::Active,
            posted_balance: Amount::ZERO,
            held_amount: Amount::ZERO,
            credit: CreditPolicy::forbid(),
            last_journal_seq: 0,
            revision: 1,
            approval: None,
        },
    );
    id
}
