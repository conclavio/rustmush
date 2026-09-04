//! The E5c verb surface (08d §5.5).
//!
//! | verb | who | what |
//! |---|---|---|
//! | `+career` | player | view; `/stance`, `/engage`, `/source add\|remove` |
//! | `+prompt` | player | view the one pending prompt; `+prompt <option>` answers it |
//! | `+resources` | player | abstract: level, strain, sources — exact: the balance |
//! | `@wod/economy` | staff | show the effective config; `migrate` |
//! | `@career/set` | staff | a character's employer and front |
//!
//! Three disclosure rules run through all of them:
//!
//! - **The number to its owner and to staff, and nothing to anybody else** (E5b fold
//!   A2, and this cut's brief). A `SupportLevel` is the Resources reading the ruleset
//!   derives from a sheet, so `+resources` shows the numeric level to **the character
//!   themself and to staff**, and refuses a non-staff third party outright — there is
//!   no assessment-word branch, because the only render that could serve one
//!   (`housing_word`) carries no view predicate and would bypass the
//!   tenant/guest/officer/staff rule `@tenancy/show` enforces (E5c fold D3).
//! - **An account is rendered under the economy's own rules.** `+resources` in exact
//!   mode asks `mush_economy::may_view_account` and prints nothing at all when the
//!   answer is no — never a placeholder, never a masked number.
//! - **A front renders only to a viewer `faction_listed` admits**, and an orphaned
//!   one renders as its raw id and never fails a read (§5.3, E5c fold D1).

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, Registrar, Role, match_object};
use mush_economy::EconomyStore;
use mush_housing::HousingStore;
use mush_org::{OrgId, PrincipalId};

use crate::economy::config::{PersonalMode, WOD_USD, WOD_USD_SCALE};
use crate::economy::prompt;
use crate::economy::record::{
    MAX_PROMPT_OPTIONS, MAX_SOURCES, STRAIN_MAX, SpendingProfile, Stance, SupportSource,
};
use crate::economy::store::{WodEconomyStore, put_profile};
use crate::splats::Dispatch;

/// Staff, by housing's own definition — one rule for the whole E-series surface.
fn is_staff(ctx: &Ctx<'_>) -> bool {
    mush_housing::is_staff(ctx.w, ctx.actor)
}

fn require_staff(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if is_staff(ctx) {
        Ok(())
    } else {
        Err(CmdError::msg("You do not have permission to do that."))
    }
}

/// The layer's own "is this on at all" gate. `off` shows nothing, per §5.1.
fn config(ctx: &Ctx<'_>) -> Option<crate::economy::WodEconomyConfig> {
    ctx.w.module_state::<WodEconomyStore>().map(|s| s.config().clone())
}

fn profile_of(ctx: &Ctx<'_>, who: ObjId) -> Option<SpendingProfile> {
    ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(who).cloned())
}

/// The cadence position, for a stamp. Zero when no clock resolves — a stamp is a
/// display, and a missing clock must not refuse a read.
fn now(ctx: &Ctx<'_>) -> i64 {
    ctx.w
        .module_state::<WodEconomyStore>()
        .and_then(WodEconomyStore::cadence)
        .and_then(|c| mush_housing::cadence_now(ctx.w, c))
        .unwrap_or(0)
}

/// The assessment word for one character's own housing, or `None` when they have no
/// abstract tenancy. Derived through housing's **public** `assess_at`, so the word
/// this prints and the word `@tenancy/show` prints cannot drift.
///
/// **It carries no view predicate of its own**, so the one caller must supply the
/// authority: `+resources` reaches it only for the actor themself or for staff (E5c
/// fold D3). Anything else asking a tenant's assessment word has to go through
/// `@tenancy/show`'s tenant/guest/officer/staff rule instead.
fn housing_word(ctx: &Ctx<'_>, who: ObjId) -> Option<&'static str> {
    let store = ctx.w.module_state::<HousingStore>()?;
    let t = store.live_tenancies_of_tenant(who).next()?;
    let plans = t.funding.plans()?;
    let tier = store.property(t.property).map(|p| p.tier).unwrap_or_default();
    let period = store.assess_state(t.id).period;
    Some(
        mush_housing::assess_at(tier, plans, &t.tenants, period, mush_housing::PLAN_MAX_AGE_PERIODS)
            .label(),
    )
}

// ---------------------------------------------------------------------------
// +career
// ---------------------------------------------------------------------------

fn career_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(cfg) = config(ctx) else {
        return Err(CmdError::msg("The WoD economy is not loaded."));
    };
    if !cfg.personal_mode.is_on() {
        return Err(CmdError::msg("This game does not run the WoD career layer."));
    }
    // `careers = false` switches the career layer off, and that has to include the
    // player verbs that drive it (E5c fold D2): a stance on a careers-off server would
    // still have bought a support tier through `level_delta`. Same wording as the
    // off-mode gate — the two refusals are the same refusal.
    if !cfg.careers && (ctx.switches.has("stance") || ctx.switches.has("engage")) {
        return Err(CmdError::msg("This game does not run the WoD career layer."));
    }
    if ctx.switches.has("stance") {
        return set_stance(ctx);
    }
    if ctx.switches.has("engage") {
        return set_engagement(ctx);
    }
    if ctx.switches.has("source") {
        return edit_source(ctx, cfg.careers);
    }
    show_career(ctx)
}

fn show_career(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let Some(p) = profile_of(ctx, me) else {
        return Err(CmdError::msg(
            "You have no support profile yet — one is created when your sheet is approved. \
             (Staff: `@wod/economy migrate` re-derives one that was lost.)",
        ));
    };
    let mut lines = vec![format!(
        "Career — support level {} · strain {}/{} · {}",
        p.level,
        p.strain,
        STRAIN_MAX,
        p.engagement.label()
    )];
    match p.career() {
        None => lines.push(
            "  No career on file. Add one with +career/source add career=<kind>.".to_string(),
        ),
        Some(c) => {
            lines.push(format!(
                "  {} — stance {} · exposure {} · time {} · visibility {} · risk {}",
                c.kind,
                c.stance.label(),
                c.stance.exposure_word(),
                c.time_demand,
                c.visibility,
                c.risk
            ));
            if let Some(next) = c.pending_stance {
                lines.push(format!("  stance {} takes effect next period.", next.label()));
            }
            match &c.front {
                None => lines.push(
                    "  No front on file — nothing is generated without one (staff: @career/set)."
                        .to_string(),
                ),
                Some(f) => lines.push(format!("  front: {}", prompt::render_front(ctx, me, f))),
            }
            if let Some(e) = &c.employer {
                lines.push(format!("  employer: {}", mush_housing::render_principal(ctx.w, e)));
            }
        }
    }
    if !p.sources.is_empty() {
        let words: Vec<String> = p.sources.iter().map(SupportSource::label).collect();
        lines.push(format!("  sources: {}", words.join(", ")));
    }
    if let Some(h) = &p.haven {
        lines.push(format!(
            "  haven ({} dot(s)): {}",
            h.dots,
            h.qualities.iter().map(smol_str::SmolStr::as_str).collect::<Vec<_>>().join(", ")
        ));
    }
    if p.prompt.is_some() {
        lines.push("  You have something waiting — type +prompt.".to_string());
    }
    for l in lines {
        ctx.out.sys_to_actor(me, "wod", l);
    }
    Ok(())
}

/// `+career/stance <coast|advance|exploit|conceal>` — applied at the **next** period
/// boundary (D-E5-6), never the current one.
fn set_stance(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let want = Stance::parse(ctx.args.raw()).ok_or_else(|| {
        CmdError::msg("Usage: +career/stance <coast|advance|exploit|conceal>")
    })?;
    let mut p = profile_of(ctx, me).ok_or_else(|| CmdError::msg("You have no support profile."))?;
    let Some(c) = p.career_mut() else {
        return Err(CmdError::msg("You have no career to take a stance in."));
    };
    if c.stance == want && c.pending_stance.is_none() {
        return Err(CmdError::msg(format!("You are already coasting on {}.", want.label())));
    }
    c.pending_stance = Some(want);
    let at = now(ctx);
    p.note(at, format!("stance {} chosen", want.label()));
    p.revision = p.revision.saturating_add(1);
    put_profile(ctx, p);
    ctx.out.info(
        me,
        format!(
            "Stance set to {}. It takes effect at the next period boundary — a flip made \
             moments before an assessment changes nothing about that one.",
            want.label()
        ),
    );
    Ok(())
}

/// `+career/engage on|off`.
fn set_engagement(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let want = crate::economy::Engagement::parse(ctx.args.raw())
        .ok_or_else(|| CmdError::msg("Usage: +career/engage on|off"))?;
    let mut p = profile_of(ctx, me).ok_or_else(|| CmdError::msg("You have no support profile."))?;
    p.engagement = want;
    p.revision = p.revision.saturating_add(1);
    put_profile(ctx, p);
    ctx.out.info(me, match want {
        crate::economy::Engagement::Engaged => {
            "Engaged: your career will bring you things to decide."
        }
        crate::economy::Engagement::Ambient => {
            "Ambient: your career runs quietly. It still supports your home and still costs \
             you strain; it simply will not ask."
        }
    });
    Ok(())
}

/// `+career/source add|remove <kind>[=<value>]`.
///
/// `careers` is the `[wod.economy].careers` switch: with it off, `career=<kind>` is
/// refused the way `/stance` and `/engage` are (E5c fold D2). The other source kinds
/// are attribution for *housing*, which `careers = false` does not switch off.
fn edit_source(ctx: &mut Ctx<'_>, careers: bool) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: +career/source add <career=<kind>|inheritance|criminal|\
                         patron=<who>|domain=<org id>>  /  +career/source remove <kind>";
    let me = ctx.actor;
    let raw = ctx.args.raw().trim().to_string();
    let (verb, rest) = raw.split_once(char::is_whitespace).unwrap_or((raw.as_str(), ""));
    let rest = rest.trim();
    let mut p = profile_of(ctx, me).ok_or_else(|| CmdError::msg("You have no support profile."))?;
    let at = now(ctx);
    match verb.to_ascii_lowercase().as_str() {
        "add" => {
            let (key, value) = rest.split_once('=').unwrap_or((rest, ""));
            let key = key.trim().to_ascii_lowercase();
            let source = match key.as_str() {
                "career" => {
                    if !careers {
                        return Err(CmdError::msg(
                            "This game does not run the WoD career layer.",
                        ));
                    }
                    if value.trim().is_empty() {
                        return Err(CmdError::msg("A career needs a kind: career=<kind>."));
                    }
                    SupportSource::Career(Box::new(crate::economy::CareerProfile::new(value)))
                }
                "inheritance" => SupportSource::Inheritance,
                "criminal" => SupportSource::Criminal,
                "patron" => {
                    let who = match_object(ctx.w, me, value.trim())
                        .map_err(|_| CmdError::msg("No such person."))?;
                    SupportSource::Patron { who: PrincipalId::Object(who) }
                }
                "domain" => {
                    // **An id, and no org-existence check** (E5c fold D5). This verb
                    // is unprivileged while the whole `@org` surface is admin-gated,
                    // so answering "no such org" — or resolving a *name* — would be
                    // exactly the org oracle E5b fold A3 closed. `SupportSource::
                    // Domain` is a lossy attribution label housing renders as the
                    // constant "domain support" and never interprets, so a phantom
                    // id authorizes nothing and discloses nothing. The refusal is a
                    // sentence rather than a bare fragment; the help says `<org id>`
                    // because that is what the parser takes.
                    let id = value
                        .trim()
                        .trim_start_matches('#')
                        .parse::<u64>()
                        .map_err(|_| {
                            CmdError::msg("A domain needs an org id: domain=<org id>.")
                        })?;
                    SupportSource::Domain { org: OrgId(id) }
                }
                // `Dependent` is reserved (D-E5-10) and nothing constructs it.
                _ => return Err(CmdError::msg(USAGE)),
            };
            if p.sources.iter().any(|s| s.key() == source.key()) {
                return Err(CmdError::msg("You already have a source of that kind."));
            }
            if p.sources.len() >= MAX_SOURCES {
                return Err(CmdError::msg(format!("At most {MAX_SOURCES} sources.")));
            }
            let label = source.label();
            p.sources.push(source);
            p.note(at, format!("source added: {label}"));
            p.revision = p.revision.saturating_add(1);
            put_profile(ctx, p);
            ctx.out.info(me, format!("Added {label}. It counts from the next period."));
            Ok(())
        }
        "remove" | "rem" | "del" => {
            let key = rest.trim().to_ascii_lowercase();
            let Some(idx) = p.sources.iter().position(|s| s.key() == key) else {
                return Err(CmdError::msg("You have no source of that kind."));
            };
            let gone = p.sources.remove(idx);
            p.note(at, format!("source removed: {}", gone.label()));
            p.revision = p.revision.saturating_add(1);
            put_profile(ctx, p);
            ctx.out.info(me, format!("Removed {}.", gone.label()));
            Ok(())
        }
        _ => Err(CmdError::msg(USAGE)),
    }
}

// ---------------------------------------------------------------------------
// +prompt
// ---------------------------------------------------------------------------

fn prompt_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let Some(cfg) = config(ctx) else {
        return Err(CmdError::msg("The WoD economy is not loaded."));
    };
    if !cfg.personal_mode.is_on() {
        return Err(CmdError::msg("This game does not run the WoD career layer."));
    }
    let choice = ctx.args.raw().trim().to_string();
    if !choice.is_empty() {
        let at = now(ctx);
        return match prompt::choose(ctx, me, &choice, at) {
            Ok(label) => {
                ctx.out.info(me, label);
                Ok(())
            }
            Err(e) => Err(CmdError::msg(e)),
        };
    }
    let Some(p) = profile_of(ctx, me) else {
        return Err(CmdError::msg("You have no support profile."));
    };
    let Some(prompt) = p.prompt.clone() else {
        ctx.out.info(me, "Nothing is waiting on you.");
        return Ok(());
    };
    let head = match &prompt.kind {
        crate::economy::PromptKind::Opportunity { front } => {
            format!("An opportunity — {}", prompt::render_front(ctx, me, front))
        }
        crate::economy::PromptKind::Advancement { front } => {
            format!("A chance to move up — {}", prompt::render_front(ctx, me, front))
        }
        crate::economy::PromptKind::HousingPressure { tenancy } => {
            format!("Your home (tenancy #{tenancy}) is behind.")
        }
        crate::economy::PromptKind::StrainRelief => {
            "You are stretched about as thin as you go.".to_string()
        }
    };
    ctx.out.sys_to_actor(me, "wod", head);
    // At most `MAX_PROMPT_OPTIONS` (look-back fold, D-LBK-3): the persisted record
    // refuses more on the way in, and the render is bounded by the same number
    // regardless of what carried it here.
    for o in prompt.options.iter().take(MAX_PROMPT_OPTIONS) {
        ctx.out.sys_to_actor(me, "wod", format!("  +prompt {}  --  {}", o.key, o.label));
    }
    // **The time actually left, from this character's own evaluation ordinal** (E5c
    // fold T6). Interpolating `expires_after_periods` — a write-once constant — made
    // every read of every prompt say "in 2 period(s)", including the last period it
    // is readable at all. Non-response is a first-class answer (owner answer 19), so
    // the time left is the one number a player needs in order to choose it
    // deliberately. This is the first player-facing read of `EvalState::period`
    // outside the tick; any later view quoting a deadline reads the same ordinal.
    let period = ctx.w.module_state::<WodEconomyStore>().map_or(0, |s| s.eval_state(me).period);
    let left = u64::from(prompt.expires_after_periods)
        .saturating_sub(period.saturating_sub(prompt.opened_period));
    let lapse = if left <= 1 {
        format!(
            "  Doing nothing is a real answer: it lapses to \"{}\" at the end of this period — \
             this is its last one — and costs you nothing.",
            prompt.default.label()
        )
    } else {
        format!(
            "  Doing nothing is a real answer: it lapses to \"{}\" in {left} period(s), and \
             costs you nothing.",
            prompt.default.label()
        )
    };
    ctx.out.sys_to_actor(me, "wod", lapse);
    Ok(())
}

// ---------------------------------------------------------------------------
// +resources
// ---------------------------------------------------------------------------

fn resources_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let Some(cfg) = config(ctx) else {
        return Err(CmdError::msg("The WoD economy is not loaded."));
    };
    if !cfg.personal_mode.is_on() {
        return Err(CmdError::msg("This game does not track personal resources."));
    }
    // A staffer may look at somebody else; anyone else sees their own.
    let target = match ctx.args.raw().trim() {
        "" => me,
        name => {
            require_staff(ctx)?;
            match_object(ctx.w, me, name)
                .map_err(|_| CmdError::msg("No such person."))?
        }
    };
    let Some(p) = profile_of(ctx, target) else {
        return Err(CmdError::msg(
            "No support profile. (Staff: `@wod/economy migrate` re-derives one that was lost.)",
        ));
    };
    // `require_staff` above means `target != me` implies staff, so **this is the only
    // branch there is**: the character themself, or a staffer (§5.6 deviation 12).
    //
    // There used to be an `else` here that printed a third party's assessment word
    // (E5c fold D3). It was unreachable — and it was a trap: `housing_word` calls
    // `live_tenancies_of_tenant` with no view predicate at all, so relaxing the gate
    // above would have handed any character a tenant's assessment word, bypassing the
    // tenant/guest/officer/staff rule `@tenancy/show` enforces (E5b fold A2). Dead
    // code that leaks the moment it is revived is worse than no code.
    ctx.out.sys_to_actor(
        me,
        "wod",
        format!(
            "Resources {} → support level {} · strain {}/{}",
            p.resources, p.level, p.strain, STRAIN_MAX
        ),
    );
    if !p.sources.is_empty() {
        let words: Vec<String> = p.sources.iter().map(SupportSource::label).collect();
        ctx.out.sys_to_actor(me, "wod", format!("  sources: {}", words.join(", ")));
    }
    if let Some(word) = housing_word(ctx, target) {
        ctx.out.sys_to_actor(me, "wod", format!("  your home assesses: {word}"));
    }
    // Exact mode only, and only through the economy's own view rule. In abstract
    // mode the account is **hidden**, not zeroed — the balance persists untouched.
    if cfg.personal_mode == PersonalMode::Exact
        && let Some(account) = p.account
        && mush_economy::may_view_account(ctx.w, me, account)
        && let Some(line) = balance_line(ctx, account)
    {
        ctx.out.sys_to_actor(me, "wod", line);
    }
    Ok(())
}

/// One balance line, rendered as `@account/show` renders it (the instrument's own
/// scale). `None` when the account has gone away.
fn balance_line(ctx: &Ctx<'_>, account: mush_economy::AccountId) -> Option<String> {
    let store = ctx.w.module_state::<EconomyStore>()?;
    let a = store.snapshot().account(account)?;
    let scale = store.snapshot().instrument(&a.instrument).map_or(WOD_USD_SCALE, |i| i.scale);
    Some(format!(
        "  account #{account} ({}): {}",
        a.instrument.as_str(),
        a.posted_balance.to_decimal_string(scale)
    ))
}

// ---------------------------------------------------------------------------
// @wod/economy, @career/set
// ---------------------------------------------------------------------------

fn wod_cmd(dispatch: &Dispatch, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !ctx.switches.has("economy") {
        return Err(CmdError::msg("Usage: @wod/economy [migrate]"));
    }
    require_staff(ctx)?;
    let Some(cfg) = config(ctx) else {
        return Err(CmdError::msg("The WoD economy is not loaded."));
    };
    // **Matched exhaustively** (E5c fold D6). The handler used to test for `migrate`
    // and treat *everything else* — a typo of the one documented subcommand, trailing
    // junk — as the bare show, so the one operation that matters was the one a slip
    // turned into a silent no-op. That is `[rulesets].enabled`'s own rule one layer
    // up: a silently-ignored typo is worse than a refusal. `@econ/cadence`, the verb
    // §5.6 deviation 13 anchors this shape to, already refuses an unrecognised word.
    let arg = ctx.args.raw().trim().to_string();
    if !arg.is_empty() {
        if !arg.eq_ignore_ascii_case("migrate") {
            return Err(CmdError::msg("Usage: @wod/economy [migrate]"));
        }
        let report = crate::economy::migrate::run(ctx, &cfg, dispatch);
        ctx.out.sys_to_actor(ctx.actor, "wod", "@wod/economy migrate:");
        for l in report.lines() {
            ctx.out.sys_to_actor(ctx.actor, "wod", l);
        }
        ctx.gate_on_durable_ack();
        return Ok(());
    }
    let me = ctx.actor;
    let (cadence, created, instrument, profiles) = ctx
        .w
        .module_state::<WodEconomyStore>()
        .map(|s| (s.cadence(), s.cadence_created(), s.instrument().cloned(), s.profiles().count()))
        .unwrap_or((None, false, None, 0));
    ctx.out.sys_to_actor(me, "wod", "[wod.economy] as this server is running it:");
    ctx.out.sys_to_actor(me, "wod", format!("  personal_mode      {}", cfg.personal_mode.label()));
    ctx.out.sys_to_actor(me, "wod", format!("  housing            {}", cfg.housing));
    ctx.out.sys_to_actor(me, "wod", format!("  careers            {}", cfg.careers));
    ctx.out.sys_to_actor(
        me,
        "wod",
        format!("  default_engagement {}", cfg.default_engagement.label()),
    );
    let clock = match cadence {
        Some(id) if created => format!("{} (#{}) — created by this ruleset's boot", cfg.cadence, id.0),
        Some(id) => format!("{} (#{}) — already existed", cfg.cadence, id.0),
        None => format!("{} — DOES NOT RESOLVE; nothing is evaluated (@econ/cadence create {})",
            cfg.cadence, cfg.cadence),
    };
    ctx.out.sys_to_actor(me, "wod", format!("  cadence            {clock}"));
    match instrument {
        Some(i) => ctx.out.sys_to_actor(me, "wod", format!("  instrument         {}", i.as_str())),
        None => ctx.out.sys_to_actor(
            me,
            "wod",
            "  instrument         none (abstract and off register no wod.usd)".to_string(),
        ),
    }
    if cfg.is_exact() {
        // The **effective** `wod.usd` cap, read live off the economy's own config
        // (per-instrument override included), so this screen and the settlement that
        // enforces it cannot disagree.
        let cap = mush_economy::InstrumentId::new(WOD_USD).and_then(|id| {
            ctx.w.module_state::<EconomyStore>().map(|s| s.config().cap_major(&id))
        });
        // An income row above the cap boots clean and pays nobody: every
        // `create_system_obligation` for that dot fails `MintCapExceeded` into a
        // `tracing::warn!` nobody reads. §5.6 does not make it a hard boot error, so
        // the screen marks it instead — the way the cadence line already marks
        // `DOES NOT RESOLVE` (E5c fold M6).
        let mut dots: Vec<String> = cfg
            .income
            .iter()
            .map(|(d, n)| match cap {
                Some(cap) if *n > cap => {
                    format!("{d}={n} (ABOVE THE MINT CAP {cap}; no line is opened)")
                }
                _ => format!("{d}={n}"),
            })
            .collect();
        if dots.is_empty() {
            dots.push("(empty — no income lines are opened)".to_string());
        }
        ctx.out.sys_to_actor(me, "wod", format!("  income (major)     {}", dots.join(" ")));
        // The chargen stake is **not** bounded by the mint cap and is not meant to be
        // (§5.6 deviation 10: it is an attributed post, not a standing mint). Saying
        // so beats implying a ceiling that does not exist (E5c fold M5).
        let stake = match cap {
            Some(cap) if cfg.starting_funds_major > cap => format!(
                "  starting funds     {} (major) (ABOVE THE MINT CAP {cap}; the stake is not \
                 capped — every approved character receives it in full)",
                cfg.starting_funds_major
            ),
            _ => format!("  starting funds     {} (major)", cfg.starting_funds_major),
        };
        ctx.out.sys_to_actor(me, "wod", stake);
        ctx.out.sys_to_actor(
            me,
            "wod",
            "  Every income LINE is bounded by [economy].mint_obligation_cap_major, re-checked \
             at every settlement. The chargen stake is bounded by starting_funds_major alone."
                .to_string(),
        );
        ctx.out.sys_to_actor(
            me,
            "wod",
            "  An income line somebody cancels by hand stays cancelled: the evaluation opens no \
             replacement until the character's Resources change or @wod/economy migrate runs."
                .to_string(),
        );
    }
    ctx.out.sys_to_actor(me, "wod", format!("  profiles           {profiles}"));
    ctx.out.sys_to_actor(
        me,
        "wod",
        "  @wod/economy migrate  --  reconcile the world to the configured mode (mints nothing)."
            .to_string(),
    );
    Ok(())
}

/// `@career/set <who>=employer=<obj|org:<id>|none> front=<faction>:<front>[:<op>] \
/// kind=<kind> time=<n> visibility=<n> risk=<n>`.
fn career_set_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !ctx.switches.has("set") {
        return Err(CmdError::msg("Usage: @career/set <who>=…"));
    }
    const USAGE: &str = "Usage: @career/set <who>=[kind=<kind>] [employer=<obj|org:<id>|none>] \
                         [front=<faction>:<front>[:<op>]|none] [time=<n>] [visibility=<n>] \
                         [risk=<n>]";
    require_staff(ctx)?;
    let me = ctx.actor;
    let who = match_object(ctx.w, me, ctx.args.lhs().trim())
        .map_err(|_| CmdError::msg("No such person."))?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
    let mut p = profile_of(ctx, who).ok_or_else(|| CmdError::msg("They have no support profile."))?;
    // A career is created on demand here, so a staffer can set a front on a character
    // who has not typed `+career/source add career=…` yet.
    if p.career().is_none() {
        p.sources.push(SupportSource::Career(Box::new(crate::economy::CareerProfile::new("work"))));
    }
    let mut changed: Vec<String> = Vec::new();
    // What to say out loud once the write has landed (E5c fold D4/D5). Deviation 5
    // deliberately permits the **set-ahead** case — a staffer may be wiring a front
    // up before the faction exists — so neither of these is a refusal; both are the
    // advisory the comment below this loop always claimed was said.
    let mut advisories: Vec<String> = Vec::new();
    for tok in rhs.split_whitespace() {
        let Some((k, v)) = tok.split_once('=') else {
            return Err(CmdError::msg(USAGE));
        };
        let v = v.trim();
        match k.trim().to_ascii_lowercase().as_str() {
            "kind" => {
                if let Some(c) = p.career_mut() {
                    c.kind = smol_str::SmolStr::new(crate::economy::record::clip(v));
                }
                changed.push(format!("kind={v}"));
            }
            "employer" => {
                let principal = parse_employer(ctx, v)?;
                // On the **staff** side there is no org oracle to protect (the whole
                // `@org` surface is admin-gated already), so an org id that resolves
                // to nothing is worth saying out loud.
                if let Some(PrincipalId::Organization(org)) = principal
                    && !org_resolves(ctx, org)
                {
                    advisories.push(format!(
                        "org:{} does not resolve; the employer is recorded as an id only.",
                        org.0
                    ));
                }
                if let Some(c) = p.career_mut() {
                    c.employer = principal;
                }
                changed.push(format!("employer={v}"));
            }
            "front" => {
                let front = parse_front(v)?;
                if let Some(f) = front
                    && !prompt::front_resolves(ctx, &f)
                {
                    advisories.push(format!(
                        "front #{}/#{} does not resolve yet; this career generates nothing until \
                         it does.",
                        f.faction.0, f.front.0
                    ));
                }
                if let Some(c) = p.career_mut() {
                    c.front = front;
                }
                changed.push(format!("front={v}"));
            }
            "time" | "visibility" | "risk" => {
                let n = v.parse::<u8>().map_err(|_| CmdError::msg(USAGE))?.min(5);
                if let Some(c) = p.career_mut() {
                    match k.trim().to_ascii_lowercase().as_str() {
                        "time" => c.time_demand = n,
                        "visibility" => c.visibility = n,
                        _ => c.risk = n,
                    }
                }
                changed.push(format!("{}={n}", k.trim()));
            }
            _ => return Err(CmdError::msg(USAGE)),
        }
    }
    if changed.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    let at = now(ctx);
    p.note(at, format!("staff set {}", changed.join(" ")));
    p.revision = p.revision.saturating_add(1);
    put_profile(ctx, p);
    ctx.out.info(me, format!("Set {} on #{}.", changed.join(", "), who.0));
    // A front (or an org employer) that does not resolve is still recorded — a
    // staffer may be setting it up ahead of the faction, deviation 5 — and it is said
    // out loud rather than silently inert. The staffer who can fix the typo is the
    // only person who would ever be told: the player's `+career` shows an
    // "(orphaned front)" they cannot act on.
    for line in advisories {
        ctx.out.info(me, line);
    }
    Ok(())
}

/// Whether an org id names a live organization (the staff-side advisory only).
fn org_resolves(ctx: &Ctx<'_>, org: OrgId) -> bool {
    ctx.w
        .module_state::<mush_org::OrgStore>()
        .is_some_and(|s| s.state().org(org).is_some())
}

fn parse_employer(ctx: &Ctx<'_>, v: &str) -> Result<Option<PrincipalId>, CmdError> {
    if v.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    if let Some(rest) = v.strip_prefix("org:") {
        let id = rest
            .trim()
            .trim_start_matches('#')
            .parse::<u64>()
            .map_err(|_| CmdError::msg("employer=org:<id>"))?;
        return Ok(Some(PrincipalId::Organization(OrgId(id))));
    }
    let who = match_object(ctx.w, ctx.actor, v)
        .map_err(|_| CmdError::msg("No such employer."))?;
    Ok(Some(PrincipalId::Object(who)))
}

fn parse_front(v: &str) -> Result<Option<crate::economy::FrontRef>, CmdError> {
    if v.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let mut parts = v.split(':');
    let faction = parts
        .next()
        .and_then(|s| s.trim().trim_start_matches('#').parse::<u64>().ok())
        .ok_or_else(|| CmdError::msg("front=<faction>:<front>[:<op>]"))?;
    let front = parts
        .next()
        .and_then(|s| s.trim().trim_start_matches('#').parse::<u64>().ok())
        .ok_or_else(|| CmdError::msg("front=<faction>:<front>[:<op>]"))?;
    let operation = parts.next().and_then(|s| s.trim().parse::<u64>().ok());
    Ok(Some(crate::economy::FrontRef {
        faction: mush_faction::FactionId(faction),
        front: mush_faction::FrontId(front),
        operation,
    }))
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register the E5c verb surface.
///
/// `dispatch` is the ruleset's sheet dispatch, captured by `@wod/economy` so
/// `migrate` reads approval through the same table `chargen_approval` stamps (E5c
/// fold M2).
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    dispatch: Arc<Dispatch>,
) -> Result<(), mush_core::ModError> {
    reg.command(
        CommandSpec {
            name: "+career".into(),
            switches: vec!["stance".into(), "engage".into(), "source".into()],
            shape: ArgShape::Raw,
            help: "+career  --  Your work, your stance, and what it is costing you.\n\
                   +career/stance <coast|advance|exploit|conceal>  --  How you are playing it. \
                   Coast is steady; Advance trades time for chances; Exploit buys capacity at \
                   the price of leaving traces; Conceal keeps your head down. A stance takes \
                   effect at the NEXT period, never the current one.\n\
                   +career/engage on|off  --  Whether your career brings you things to decide \
                   (off is quiet: it still supports your home and still costs strain).\n\
                   +career/source add career=<kind>|inheritance|criminal|patron=<who>|\
                   domain=<org id>  --  Record why you can afford your life.\n\
                   +career/source remove <kind>  --  Drop one.\n\
                   Nothing here changes a sheet dot. Raise Resources with +xp/spend."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(career_cmd),
    )?;
    reg.command(
        CommandSpec {
            name: "+prompt".into(),
            shape: ArgShape::Raw,
            help: "+prompt  --  The one thing waiting on you, if anything is.\n\
                   +prompt <option>  --  Answer it.\n\
                   You are never asked two things at once, and doing nothing is a real answer: \
                   a prompt lapses to its stated default and costs you nothing — no strain, no \
                   money, no support."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(prompt_cmd),
    )?;
    reg.command(
        CommandSpec {
            name: "+resources".into(),
            shape: ArgShape::Raw,
            help: "+resources  --  What your background does for you: your support level, \
                   your strain, and where it comes from. In an exact-cash game it also shows \
                   your balance.\n\
                   +resources <who>  --  Staff: somebody else's."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(resources_cmd),
    )?;
    reg.command(
        CommandSpec {
            name: "@wod".into(),
            switches: vec!["economy".into()],
            shape: ArgShape::Raw,
            min_role: Role::Royalty,
            help: "@wod/economy  --  The effective [wod.economy] configuration, the campaign \
                   clock (and whether this boot created it), and the instrument.\n\
                   @wod/economy migrate  --  One-shot reconciliation after a personal_mode \
                   flip. It lists what it did, including every character it skipped. Its \
                   population is the APPROVED sheets — a character still in chargen gets \
                   their artifacts at approval. abstract->exact opens zero-balance accounts \
                   (nothing is minted); exact->abstract stops the income lines and hides the \
                   wallets, leaving every balance untouched; off stops evaluating and deletes \
                   nothing. Anything but `migrate` is refused."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| wod_cmd(&dispatch, ctx)),
    )?;
    reg.command(
        CommandSpec {
            name: "@career".into(),
            switches: vec!["set".into()],
            shape: ArgShape::EqPair,
            min_role: Role::Royalty,
            help: "@career/set <who>=[kind=<kind>] [employer=<obj|org:<id>|none>] \
                   [front=<faction>:<front>[:<op>]|none] [time=<n>] [visibility=<n>] [risk=<n>] \
                   --  Staff: a character's career, its employer, and the front its progress \
                   is attributed to. Without a front the career generates nothing at all — \
                   every opportunity and every piece of evidence has to cite one."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(career_set_cmd),
    )?;
    Ok(())
}
