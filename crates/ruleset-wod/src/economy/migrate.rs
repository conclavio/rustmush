//! `@wod/economy migrate` — the one-shot staff reconciliation after a
//! `personal_mode` flip (08d §5.1, D-E5-9).
//!
//! Flipping the mode is editing `mushd.toml` and restarting; **this** is the act that
//! reconciles the world to it, and it **lists what it did**. The table §5.1 pins,
//! implemented exactly:
//!
//! | flip | what migrate does | what it never does |
//! |---|---|---|
//! | abstract → exact | opens a **zero-balance** `wod.usd` account per profiled character | mints anything (staff seed with an attributed `@econ/mint`) |
//! | exact → abstract | creates a `SpendingProfile` per character from Resources, stops every income line, and **hides** `wod.usd` from `+resources` | touch an account or a balance — they are economy infrastructure and persist untouched |
//! | `housing = false` | stops **new** tenancies | end, alter or hide an existing one |
//! | `off` | stops evaluating | delete a profile |
//!
//! One line matters more than the rest: **it mints nothing, in either direction.** An
//! account opened by a migration has a zero balance, and the only way it acquires one
//! is a staff `@econ/mint` somebody signed their name to.

use mush_core::{Ctx, ObjId};

use crate::economy::boot;
use crate::economy::config::{PersonalMode, WodEconomyConfig};
use crate::economy::record::SpendingProfile;
use crate::economy::store::{WodEconomyStore, put_profile, seed_eval};

/// What one reconciliation did, in the order the report prints it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Profiles created from a character's sheet (exact → abstract, or first run).
    pub profiles_created: Vec<ObjId>,
    /// Zero-balance `wod.usd` accounts opened (abstract → exact).
    pub accounts_opened: Vec<ObjId>,
    /// Income lines stopped (→ abstract / off).
    pub income_stopped: Vec<ObjId>,
    /// Profiles whose `wod.usd` account is now hidden from `+resources`.
    pub accounts_hidden: Vec<ObjId>,
    /// Characters skipped because something refused; each line says why.
    pub skipped: Vec<String>,
}

impl Report {
    /// Whether anything at all happened.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.profiles_created.is_empty()
            && self.accounts_opened.is_empty()
            && self.income_stopped.is_empty()
            && self.accounts_hidden.is_empty()
            && self.skipped.is_empty()
    }

    /// The lines `@wod/economy migrate` prints.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        let render = |ids: &[ObjId]| -> String {
            ids.iter().map(|o| format!("#{}", o.0)).collect::<Vec<_>>().join(", ")
        };
        if !self.profiles_created.is_empty() {
            out.push(format!(
                "  {} profile(s) created from Resources: {}",
                self.profiles_created.len(),
                render(&self.profiles_created)
            ));
        }
        if !self.accounts_opened.is_empty() {
            out.push(format!(
                "  {} zero-balance wod.usd account(s) opened: {}",
                self.accounts_opened.len(),
                render(&self.accounts_opened)
            ));
            out.push(
                "  Nothing was minted. Seed a character with an attributed @econ/mint."
                    .to_string(),
            );
        }
        if !self.income_stopped.is_empty() {
            out.push(format!(
                "  {} income line(s) stopped: {}",
                self.income_stopped.len(),
                render(&self.income_stopped)
            ));
        }
        if !self.accounts_hidden.is_empty() {
            out.push(format!(
                "  {} wod.usd account(s) are now hidden from +resources (balances untouched): {}",
                self.accounts_hidden.len(),
                render(&self.accounts_hidden)
            ));
        }
        for s in &self.skipped {
            out.push(format!("  skipped: {s}"));
        }
        if out.is_empty() {
            out.push("  Nothing to do — the world already matches the configured mode.".to_string());
        }
        out
    }
}

/// Reconcile the world to the configured mode. Idempotent: running it twice does
/// nothing the second time.
pub fn run(ctx: &mut Ctx<'_>, cfg: &WodEconomyConfig) -> Report {
    let mut report = Report::default();
    match cfg.personal_mode {
        // `off` stops evaluating and **deletes nothing** (D-E5-9). It still stops the
        // standing mints, because an income obligation the scheduler executes forever
        // in a deployment that has switched the layer off is money nobody decided to
        // pay.
        PersonalMode::Off => stop_income(ctx, &mut report),
        PersonalMode::Abstract => {
            create_profiles(ctx, cfg, &mut report);
            stop_income(ctx, &mut report);
            hide_accounts(ctx, &mut report);
        }
        PersonalMode::Exact => {
            create_profiles(ctx, cfg, &mut report);
            open_accounts(ctx, &mut report);
        }
    }
    report
}

/// Every character who has a WoD sheet — the population a migration reconciles.
///
/// A sheet is the membership test rather than "every Player": a staff bot, an NPC
/// shell or an unfinished account has no Resources to derive anything from, and a
/// profile for one would be a record about nobody.
fn sheeted(ctx: &Ctx<'_>) -> Vec<ObjId> {
    ctx.w
        .iter()
        .filter(|o| o.kind == mush_core::ObjKind::Player)
        .map(|o| o.id)
        .filter(|id| ctx.w.attr(*id, crate::economy::SHEET_ATTR).is_some())
        .collect()
}

/// Give every sheeted character a profile if they have none (§5.1's "creates a
/// `SpendingProfile` per character from Resources").
fn create_profiles(ctx: &mut Ctx<'_>, cfg: &WodEconomyConfig, report: &mut Report) {
    for who in sheeted(ctx) {
        let has = ctx.w.module_state::<WodEconomyStore>().is_some_and(|s| s.profile(who).is_some());
        if has {
            continue;
        }
        let resources = crate::economy::sheet_resources(ctx.w, who).unwrap_or(0);
        let mut p = SpendingProfile::new(who, resources, cfg.default_engagement);
        p.note(0, "profile created by @wod/economy migrate");
        put_profile(ctx, p);
        // The cursor is seeded at the clock's current position, so the next period
        // the clock advances is the first one this character is evaluated on.
        seed_eval(ctx, who);
        report.profiles_created.push(who);
    }
}

/// Open a **zero-balance** account per profiled character (abstract → exact).
fn open_accounts(ctx: &mut Ctx<'_>, report: &mut Report) {
    let Some(instrument) =
        ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.instrument().cloned())
    else {
        report.skipped.push("wod.usd is not registered; no accounts were opened".to_string());
        return;
    };
    for who in ctx.w.module_state::<WodEconomyStore>().map(WodEconomyStore::profiled).unwrap_or_default()
    {
        let already = ctx
            .w
            .module_state::<WodEconomyStore>()
            .and_then(|s| s.profile(who).and_then(|p| p.account))
            .is_some();
        if already {
            continue;
        }
        let existing = boot::personal_account(ctx, who, &instrument);
        let Some(id) = existing.or_else(|| boot::open_personal_account(ctx, who, &instrument))
        else {
            report.skipped.push(format!("#{}: no account could be opened", who.0));
            continue;
        };
        if let Some(mut p) =
            ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(who).cloned())
        {
            p.account = Some(id);
            p.revision = p.revision.saturating_add(1);
            p.note(0, "wod.usd account opened by @wod/economy migrate (zero balance)");
            put_profile(ctx, p);
        }
        if existing.is_none() {
            report.accounts_opened.push(who);
        }
    }
}

/// Stop every income line this layer opened (→ abstract / off).
///
/// The **account and its balance are untouched** — they are economy infrastructure
/// (§5.1). What stops is the standing mint, because a faucet-funded obligation the
/// scheduler executes forever in a deployment that no longer shows wallets is money
/// nobody decided to pay.
fn stop_income(ctx: &mut Ctx<'_>, report: &mut Report) {
    let Some(faucet) = boot::principal(crate::economy::config::FAUCET_INCOME) else { return };
    for who in ctx.w.module_state::<WodEconomyStore>().map(WodEconomyStore::profiled).unwrap_or_default()
    {
        let Some(id) = ctx
            .w
            .module_state::<WodEconomyStore>()
            .and_then(|s| s.profile(who).and_then(|p| p.income))
        else {
            continue;
        };
        match mush_economy::set_system_obligation(ctx, &faucet, id, None) {
            Ok(()) => report.income_stopped.push(who),
            Err(error) => report.skipped.push(format!("#{}: {error}", who.0)),
        }
        if let Some(mut p) =
            ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(who).cloned())
        {
            p.income = None;
            p.revision = p.revision.saturating_add(1);
            put_profile(ctx, p);
        }
    }
}

/// Note which profiles still name an account that `+resources` will now hide.
///
/// It **clears nothing**: the pointer stays so a later flip back to exact finds the
/// same wallet instead of opening a second one, and `+resources` hides it because the
/// *mode* says so, not because the record was edited.
fn hide_accounts(ctx: &mut Ctx<'_>, report: &mut Report) {
    for who in ctx.w.module_state::<WodEconomyStore>().map(WodEconomyStore::profiled).unwrap_or_default()
    {
        let has = ctx
            .w
            .module_state::<WodEconomyStore>()
            .and_then(|s| s.profile(who).and_then(|p| p.account))
            .is_some();
        if has {
            report.accounts_hidden.push(who);
        }
    }
}
