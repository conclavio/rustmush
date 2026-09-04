//! E5c boot: resolve (or create) the campaign clock, and — in exact mode only —
//! register `wod.usd` and the three system principals' accounts (08d §5.1).
//!
//! `off` **creates nothing**: no instrument, no principal, no account, no clock.
//! `abstract` creates the clock and nothing else — there is no `wod.usd` in an
//! abstract deployment, so a character never sees a meaningless zero-dollar wallet
//! (owner answers 2, 3, 15).
//!
//! Everything here is idempotent: it is written to run on every boot, find what the
//! last one left, and add only what is missing.

use mush_core::Ctx;
use mush_economy::{
    Account, AccountStatus, Amount, CadenceSource, CreditPolicy, EconomyStore, InstrumentDef,
    InstrumentId, InstrumentKind, InstrumentStatus, NegativePolicy,
};
use mush_org::{PrincipalId, SystemPrincipalId};
use smol_str::SmolStr;

use crate::economy::config::{
    FAUCET_CHARGEN, FAUCET_HOUSING, FAUCET_INCOME, PersonalMode, WOD_USD, WOD_USD_SCALE,
    WodEconomyConfig,
};
use crate::economy::store::WodEconomyStore;

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

/// Run the whole boot for `cfg`, in order: clock, then instrument, then accounts.
pub fn run(ctx: &mut Ctx<'_>, cfg: &WodEconomyConfig) {
    if !cfg.personal_mode.is_on() && !cfg.housing {
        // `off` with housing off: the layer exists and does nothing. The clock is
        // still worth resolving so `@wod/economy` can say what it would use.
        resolve_cadence(ctx, cfg, false);
        return;
    }
    resolve_cadence(ctx, cfg, true);
    if cfg.personal_mode == PersonalMode::Exact {
        let Some(instrument) = ensure_instrument(ctx) else {
            return;
        };
        if let Some(store) = ctx.w.module_state_mut::<WodEconomyStore>() {
            store.instrument = Some(instrument.clone());
        }
        for name in faucets() {
            ensure_system_account(ctx, name, &instrument);
        }
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
    mush_economy::register_instrument(ctx, InstrumentDef {
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
    });
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
