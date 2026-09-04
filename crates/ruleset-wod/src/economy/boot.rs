//! E5c boot: resolve (or create) the campaign clock, and — in exact mode only —
//! register `wod.usd` and the three system principals' accounts (08d §5.1).
//!
//! `off` creates one inert `Manual` clock so housing can assess; nothing else — no
//! instrument, no principal, no account (E5c-C2: a manual clock advances only by an
//! explicit act, so it charges and evaluates nothing on its own). `abstract` creates
//! the clock and nothing else — there is no `wod.usd` in an abstract deployment, so a
//! character never sees a meaningless zero-dollar wallet (owner answers 2, 3, 15).
//!
//! Everything here is idempotent: it is written to run on every boot, find what the
//! last one left, and add only what is missing.

use std::collections::BTreeSet;

use mush_core::{Ctx, ObjId};
use mush_economy::{
    Account, AccountId, AccountStatus, Amount, CadenceSource, CreditPolicy, EconomyStore,
    InstrumentDef, InstrumentId, InstrumentKind, InstrumentStatus, NegativePolicy, Obligation,
    ObligationCause, ObligationId, ObligationStatusKind,
};
use mush_org::{PrincipalId, SystemPrincipalId};
use smol_str::SmolStr;

use crate::economy::config::{
    FAUCET_CHARGEN, FAUCET_HOUSING, FAUCET_INCOME, PersonalMode, WOD_USD, WOD_USD_SCALE,
    WodEconomyConfig,
};
use crate::economy::record::SpendingProfile;
use crate::economy::store::{WodEconomyStore, put_profile, seed_eval};

/// The three system principals E5c names (§5.1).
#[must_use]
pub fn faucets() -> [&'static str; 3] {
    [FAUCET_INCOME, FAUCET_HOUSING, FAUCET_CHARGEN]
}

/// A well-formed [`SystemPrincipalId`] for one of ours.
///
/// The three names are compile-time literals in `[a-z0-9_.-]`, so this cannot fail
/// for any caller in this crate; a `None` would mean somebody added a fourth name
/// with a capital letter in it, and the callers all treat it as "do nothing".
#[must_use]
pub fn principal(name: &str) -> Option<SystemPrincipalId> {
    SystemPrincipalId::new(name)
}

/// Run the whole boot for `cfg`, in order: clock, then instrument, then accounts,
/// then the two **reconciliation passes** the verify fold added.
pub fn run(ctx: &mut Ctx<'_>, cfg: &WodEconomyConfig) {
    if !cfg.personal_mode.is_on() && !cfg.housing {
        // `off` with housing off: the layer exists and does nothing. The clock is
        // still worth resolving so `@wod/economy` can say what it would use, and it
        // is not created here (an `off` world with housing on reaches the branch
        // below, which creates the one inert Manual clock housing assesses on).
        resolve_cadence(ctx, cfg, false);
        return;
    }
    resolve_cadence(ctx, cfg, true);
    if cfg.personal_mode == PersonalMode::Exact
        && let Some(instrument) = ensure_instrument(ctx)
    {
        if let Some(store) = ctx.w.module_state_mut::<WodEconomyStore>() {
            store.instrument = Some(instrument.clone());
        }
        for name in faucets() {
            ensure_system_account(ctx, name, &instrument);
        }
        warn_over_cap_stake(ctx, cfg, &instrument);
    }
    // E5c fold T1 — make `store`'s "skipped loudly and re-derived" true.
    rederive_lost_profiles(ctx, cfg);
    // E5c fold M3 — a standing mint no profile names is adopted or cancelled here.
    if cfg.personal_mode == PersonalMode::Exact {
        reconcile_income_lines(ctx);
    }
}

/// Rebuild a [`SpendingProfile`] for every character who plainly had one and does
/// not now (E5c fold T1).
///
/// The loader skips a `profile:{obj}` it cannot parse (or one whose `obj` disagrees
/// with its key) and says so — but the evaluation's working set is
/// [`WodEconomyStore::profiled`], so nothing would ever re-derive it: the character
/// leaves the economy silently, submits no plan, and with `UNKNOWN_GRACE` at 1 their
/// tenancy walks to the story window. That is precisely the loss §5.6 promised costs
/// the home *nothing*.
///
/// Two unambiguous "this profile was lost" signals: an `eval:{obj}` cursor with no
/// profile beside it (only a profiled character is ever given one), and membership of
/// a **live tenancy** (housing is asking this character for a plan every period). The
/// rebuild is `migrate::create_profiles`' exactly — the sheet's Resources, the
/// configured engagement, a note — and the cursor is re-seeded at the clock's current
/// position so the character is not charged for the periods they were missing.
fn rederive_lost_profiles(ctx: &mut Ctx<'_>, cfg: &WodEconomyConfig) {
    if !cfg.personal_mode.is_on() {
        return;
    }
    let mut want: BTreeSet<ObjId> = ctx
        .w
        .module_state::<WodEconomyStore>()
        .map(|s| s.eval.keys().copied().collect())
        .unwrap_or_default();
    if let Some(housing) = ctx.w.module_state::<mush_housing::HousingStore>() {
        for t in housing.tenancies().filter(|t| t.status.is_live()) {
            want.extend(t.tenants.iter().copied());
        }
    }
    let lost: Vec<ObjId> = want
        .into_iter()
        .filter(|who| {
            ctx.w.module_state::<WodEconomyStore>().is_some_and(|s| s.profile(*who).is_none())
        })
        // Existence **plus** not-`GOING`: a husk on its way out is not re-profiled
        // (the house liveness predicate, inline — E5c fold T4).
        .filter(|who| {
            ctx.w.get(*who).is_some_and(|o| !o.flags.contains(mush_core::Flags::GOING))
        })
        .collect();
    let instrument = ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.instrument().cloned());
    for who in lost {
        let resources = crate::economy::sheet_resources(ctx.w, who).unwrap_or(0);
        let mut p = SpendingProfile::new(who, resources, cfg.default_engagement);
        // The wallet is not derived — it exists, owned by this character in
        // `wod.usd` — so a re-derivation finds it rather than leaving the profile
        // pointing at nothing (which would make the income reconciliation below read
        // their live line as unclaimed and cancel it).
        if let Some(instrument) = &instrument {
            p.account = personal_account(ctx, who, instrument);
        }
        p.note(0, "profile re-derived at boot (the stored record was lost)");
        put_profile(ctx, p);
        seed_eval(ctx, who);
        tracing::warn!(
            who = who.0, resources,
            "wod.economy: re-derived a lost spending profile at boot (strain and any open prompt \
             are gone; the support level and the home are not)"
        );
    }
}

/// Whether `o` is one of **this layer's** standing income mints, and still live.
///
/// All three facts live on the obligation itself — the `wod.income` cause namespace,
/// the faucet that minted it, and a status that is neither `Cancelled` nor
/// `Satisfied` — which is the whole reason a lost `SpendingProfile::income` pointer is
/// recoverable (E5c fold M3). **The record is self-identifying; the pointer is a
/// cache.** Any later layer that mints under a faucet owes itself the same property.
#[must_use]
pub fn is_live_income_line(o: &Obligation) -> bool {
    let ours = matches!(
        &o.cause,
        ObligationCause::Ruleset { namespace } if namespace.as_str() == FAUCET_INCOME
    );
    let minted = principal(FAUCET_INCOME)
        .is_some_and(|f| o.minted_by.as_ref() == Some(&PrincipalId::System(f)));
    let live = !matches!(
        o.status.kind(),
        ObligationStatusKind::Cancelled | ObligationStatusKind::Satisfied
    );
    ours && minted && live
}

/// Every live income line this layer minted, as `(id, credit account)`.
#[must_use]
pub fn live_income_lines(ctx: &Ctx<'_>) -> Vec<(ObligationId, AccountId)> {
    ctx.w
        .module_state::<EconomyStore>()
        .map(|s| {
            s.snapshot()
                .obligations()
                .filter(|o| is_live_income_line(o))
                .map(|o| (o.id, o.credit_account))
                .collect()
        })
        .unwrap_or_default()
}

/// Adopt or cancel every live `wod.income` line no profile names (E5c fold M3).
///
/// The third pass of its kind, after 08c §4.8's B2 orphan-escrow sweep and E5b's
/// tenancy reconciliation, and it exists for the same reason: a pointer that is the
/// *only* name for a record strands the record the moment the pointer is lost. A line
/// crediting a profiled character whose `income` is `None` is **adopted**; a line
/// crediting nobody the layer knows about is **cancelled**; both are logged.
fn reconcile_income_lines(ctx: &mut Ctx<'_>) {
    let Some(faucet) = principal(FAUCET_INCOME) else {
        return;
    };
    let named: BTreeSet<ObligationId> = ctx
        .w
        .module_state::<WodEconomyStore>()
        .map(|s| s.profiles().filter_map(|p| p.income).collect())
        .unwrap_or_default();
    for (id, credit) in live_income_lines(ctx) {
        if named.contains(&id) {
            continue;
        }
        let owner = ctx.w.module_state::<WodEconomyStore>().and_then(|s| {
            s.profiles().find(|p| p.account == Some(credit) && p.income.is_none()).map(|p| p.obj)
        });
        match owner {
            Some(who) => {
                if let Some(mut p) =
                    ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(who).cloned())
                {
                    p.income = Some(id);
                    p.revision = p.revision.saturating_add(1);
                    p.note(0, format!("income line #{} adopted at boot", id.0));
                    put_profile(ctx, p);
                }
                tracing::warn!(
                    obligation = id.0, who = who.0,
                    "wod.economy: adopted an orphaned income line at boot (the profile had lost \
                     its pointer; the record named itself)"
                );
            }
            None => {
                match mush_economy::set_system_obligation(ctx, &faucet, id, None) {
                    Ok(()) => tracing::warn!(
                        obligation = id.0, credit = credit.0,
                        "wod.economy: cancelled an income line no profile claims"
                    ),
                    Err(error) => tracing::error!(
                        obligation = id.0, credit = credit.0, %error,
                        "wod.economy: could not cancel an unclaimed income line; it is still \
                         minting (staff: @obligation/cancel)"
                    ),
                }
            }
        }
    }
}

/// A chargen stake bigger than the mint cap is **allowed**, and is said out loud
/// (E5c fold M5).
///
/// `starting_funds_major` is an ordinary attributed two-line `post` (§5.6 deviation
/// 10), so `[economy].mint_obligation_cap_major` — a *standing-mint* authority — does
/// not bound it and is not meant to. This never refuses (deviation 11: the approval
/// never fails); it warns once at boot, and `@wod/economy` marks the line.
fn warn_over_cap_stake(ctx: &Ctx<'_>, cfg: &WodEconomyConfig, instrument: &InstrumentId) {
    let Some(cap) = ctx.w.module_state::<EconomyStore>().map(|s| s.config().cap_major(instrument))
    else {
        return;
    };
    if cfg.starting_funds_major > cap {
        tracing::warn!(
            starting_funds_major = cfg.starting_funds_major, mint_cap_major = cap,
            "wod.economy: [wod.economy].starting_funds_major is larger than \
             [economy].mint_obligation_cap_major. That is allowed — the chargen stake is an \
             attributed post, not a standing mint, and the cap bounds income LINES only — but \
             every approved character receives it in full"
        );
    }
}

/// Find the configured cadence clock by **name**, or create it (08d §5.1, note: the
/// clock must exist at boot or be created here under staff-equivalent authority).
///
/// A ruleset's boot is exactly the caller `mush_economy::create_cadence` exists for:
/// there is nobody to type `@econ/cadence create` before the first period is
/// assessed, and a campaign clock that does not resolve freezes every tenancy on it
/// (E5b fold T6). `@wod/economy` reports which of the two happened.
fn resolve_cadence(ctx: &mut Ctx<'_>, cfg: &WodEconomyConfig, create: bool) {
    let existing = ctx.w.module_state::<EconomyStore>().and_then(|s| {
        s.snapshot()
            .cadences()
            .find(|c| c.name.eq_ignore_ascii_case(cfg.cadence.as_str()))
            .map(|c| c.id)
    });
    if let Some(id) = existing {
        if let Some(store) = ctx.w.module_state_mut::<WodEconomyStore>() {
            store.cadence = Some(id);
            store.cadence_created = false;
        }
        return;
    }
    if !create {
        return;
    }
    // `Manual` by default is deliberate and is the safe direction: a clock nobody
    // advances assesses nothing, while a real- or IC-sourced clock created with the
    // wrong period would start charging periods against every profile on the next
    // sweep. Staff repoint it with `@econ/cadence` once, and this boot then finds it.
    match mush_economy::create_cadence(ctx, cfg.cadence.as_str(), CadenceSource::Manual, 1, None) {
        Ok(id) => {
            tracing::info!(
                cadence = %cfg.cadence, id = id.0,
                "wod.economy: created the campaign cadence clock named in [wod.economy].cadence \
                 (source=manual; repoint it with @econ/cadence if the campaign runs on real or \
                 IC time)"
            );
            if let Some(store) = ctx.w.module_state_mut::<WodEconomyStore>() {
                store.cadence = Some(id);
                store.cadence_created = true;
            }
        }
        Err(error) => {
            tracing::error!(
                cadence = %cfg.cadence, %error,
                "wod.economy: could not create the campaign cadence clock; nothing will be \
                 evaluated until one exists (@econ/cadence create)"
            );
        }
    }
}

/// Register `wod.usd` at scale 2 if it is not already there (owner answer 2).
fn ensure_instrument(ctx: &mut Ctx<'_>) -> Option<InstrumentId> {
    let id = InstrumentId::new(WOD_USD)?;
    let known = ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(|s| s.snapshot().instrument(&id).is_some());
    if known {
        return Some(id);
    }
    if let Err(error) = mush_economy::register_instrument(ctx, InstrumentDef {
        id: id.clone(),
        ruleset: SmolStr::new_static("wod"),
        code: SmolStr::new_static(WOD_USD),
        kind: InstrumentKind::Currency,
        scale: WOD_USD_SCALE,
        transferable: true,
        // A faucet must be able to run negative — that is what a faucet *is* — and
        // `SystemOnly` is the policy `@instrument/register` gives every currency, so
        // an ordinary character's account still cannot.
        negative_policy: NegativePolicy::SystemOnly,
        status: InstrumentStatus::Active,
        revision: 1,
    }) {
        // Create-only since the look-back fold (E2-C18); the `known` check above
        // makes this unreachable, and a refusal here still leaves the registered
        // instrument exactly as it was.
        tracing::warn!(%error, "wod.economy: wod.usd was already registered under this boot");
    }
    Some(id)
}

/// Open a system principal's `wod.usd` account if it has none.
///
/// The faucet **must own its debit account** for `create_system_obligation` to
/// accept it (lock 1, 08c §4.7 M1), and only an account opened here or by
/// `@account/system` satisfies that.
fn ensure_system_account(ctx: &mut Ctx<'_>, name: &str, instrument: &InstrumentId) {
    let Some(sys) = principal(name) else {
        return;
    };
    let owner = PrincipalId::System(sys);
    let known = ctx.w.module_state::<EconomyStore>().is_some_and(|s| {
        s.snapshot().accounts().any(|a| a.owner == owner && a.instrument == *instrument)
    });
    if known {
        return;
    }
    let Some(id) = ctx.w.module_state_mut::<EconomyStore>().and_then(EconomyStore::alloc_account)
    else {
        tracing::error!(
            principal = name,
            "wod.economy: the account-id allocator is exhausted; no faucet account was opened"
        );
        return;
    };
    mush_economy::open_account(ctx, Account {
        id,
        owner,
        instrument: instrument.clone(),
        scope: None,
        name: SmolStr::new(name),
        status: AccountStatus::Active,
        posted_balance: Amount::ZERO,
        held_amount: Amount::ZERO,
        last_hold_seq: 0,
        // A faucet is a source: unbounded negative is what makes it one, and the
        // instrument's `SystemOnly` policy is what keeps that privilege system-side.
        credit: CreditPolicy::unbounded(),
        last_journal_seq: 0,
        revision: 1,
        approval: None,
    });
    tracing::info!(principal = name, account = id.0, "wod.economy: opened a system account");
}

/// The account owned by system principal `name` in `wod.usd`, if any.
#[must_use]
pub fn system_account(
    ctx: &Ctx<'_>,
    name: &str,
    instrument: &InstrumentId,
) -> Option<mush_economy::AccountId> {
    let sys = principal(name)?;
    let owner = PrincipalId::System(sys);
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .accounts()
        .find(|a| a.owner == owner && a.instrument == *instrument)
        .map(|a| a.id)
}

/// The `wod.usd` account owned by `who`, if any.
#[must_use]
pub fn personal_account(
    ctx: &Ctx<'_>,
    who: mush_core::ObjId,
    instrument: &InstrumentId,
) -> Option<mush_economy::AccountId> {
    let owner = PrincipalId::Object(who);
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .accounts()
        .find(|a| a.owner == owner && a.instrument == *instrument)
        .map(|a| a.id)
}

/// Open a personal `wod.usd` account for `who` (exact mode; chargen and `migrate`).
///
/// Returns the existing one when there already is one, so both callers are
/// idempotent and neither can give one character two wallets.
pub fn open_personal_account(
    ctx: &mut Ctx<'_>,
    who: mush_core::ObjId,
    instrument: &InstrumentId,
) -> Option<mush_economy::AccountId> {
    if let Some(id) = personal_account(ctx, who, instrument) {
        return Some(id);
    }
    let id = ctx.w.module_state_mut::<EconomyStore>().and_then(EconomyStore::alloc_account)?;
    let name = ctx
        .w
        .get(who)
        .map_or_else(|| format!("#{}", who.0), |o| {
            mush_core::output::primary_name(&o.name).to_string()
        });
    mush_economy::open_account(ctx, Account {
        id,
        owner: PrincipalId::Object(who),
        instrument: instrument.clone(),
        scope: None,
        name: SmolStr::new(crate::economy::record::clip(&name)),
        status: AccountStatus::Active,
        posted_balance: Amount::ZERO,
        held_amount: Amount::ZERO,
        last_hold_seq: 0,
        credit: CreditPolicy::forbid(),
        last_journal_seq: 0,
        revision: 1,
        approval: None,
    });
    Some(id)
}

/// Convert a **major**-unit config number into an [`Amount`] at `wod.usd`'s scale,
/// exactly as `EconomyConfig::mint_cap` does (08c §4.7 M1's reference conversion).
///
/// `None` when the instrument is unknown or the number does not fit — an
/// unrepresentable amount refuses, it is never silently clamped into one that mints.
#[must_use]
pub fn major_to_amount(ctx: &Ctx<'_>, instrument: &InstrumentId, major: u64) -> Option<Amount> {
    let scale = ctx.w.module_state::<EconomyStore>()?.snapshot().instrument(instrument)?.scale;
    let factor = 10_i128.checked_pow(u32::from(scale))?;
    let minor = i128::from(major).checked_mul(factor)?;
    Amount::from_widened(minor)
}
