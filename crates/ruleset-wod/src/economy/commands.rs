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
//! - **The assessment word, not the number** (E5b fold A2, and this cut's brief). A
//!   `SupportLevel` is the Resources reading the ruleset derives from a sheet, so
//!   `+resources` shows the numeric level to **the character themself and to staff**,
//!   and the word (`supported` / `strained` / `unsupported` / `unknown`) to anybody
//!   else — the same split `@tenancy/show` enforces.
//! - **An account is rendered under the economy's own rules.** `+resources` in exact
//!   mode asks `mush_economy::may_view_account` and prints nothing at all when the
//!   answer is no — never a placeholder, never a masked number.
//! - **An orphaned front renders as its raw id** and never fails a read (§5.3).

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, Registrar, Role, match_object};
use mush_economy::EconomyStore;
use mush_housing::HousingStore;
use mush_org::{OrgId, PrincipalId};

use crate::economy::config::{PersonalMode, WOD_USD_SCALE};
use crate::economy::prompt;
use crate::economy::record::{
    MAX_SOURCES, STRAIN_MAX, SpendingProfile, Stance, SupportSource,
};
use crate::economy::store::{WodEconomyStore, put_profile};

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
    if ctx.switches.has("stance") {
        return set_stance(ctx);
    }
    if ctx.switches.has("engage") {
        return set_engagement(ctx);
    }
    if ctx.switches.has("source") {
        return edit_source(ctx);
    }
    show_career(ctx)
}

fn show_career(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let Some(p) = profile_of(ctx, me) else {
        return Err(CmdError::msg(
            "You have no support profile yet — one is created when your sheet is approved.",
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
                Some(f) => lines.push(format!("  front: {}", prompt::render_front(ctx, f))),
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
fn edit_source(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
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
                    let id = value
                        .trim()
                        .trim_start_matches('#')
                        .parse::<u64>()
                        .map_err(|_| CmdError::msg("domain=<org id>."))?;
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
            format!("An opportunity — {}", prompt::render_front(ctx, front))
        }
        crate::economy::PromptKind::Advancement { front } => {
            format!("A chance to move up — {}", prompt::render_front(ctx, front))
        }
        crate::economy::PromptKind::HousingPressure { tenancy } => {
            format!("Your home (tenancy #{tenancy}) is behind.")
        }
        crate::economy::PromptKind::StrainRelief => {
            "You are stretched about as thin as you go.".to_string()
        }
    };
    ctx.out.sys_to_actor(me, "wod", head);
    for o in &prompt.options {
        ctx.out.sys_to_actor(me, "wod", format!("  +prompt {}  --  {}", o.key, o.label));
    }
    ctx.out.sys_to_actor(
        me,
        "wod",
        format!(
            "  Doing nothing is a real answer: it lapses to \"{}\" in {} period(s), and costs \
             you nothing.",
            prompt.default.label(),
            prompt.expires_after_periods
        ),
    );
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
        return Err(CmdError::msg("No support profile."));
    };
    // The character themself and staff see the number; anybody else sees the word
    // (E5b fold A2 — a `SupportLevel` is a sheet reading).
    let own = target == me || is_staff(ctx);
    if own {
        ctx.out.sys_to_actor(
            me,
            "wod",
            format!(
                "Resources {} → support level {} · strain {}/{}",
                p.resources, p.level, p.strain, STRAIN_MAX
            ),
        );
    } else if let Some(word) = housing_word(ctx, target) {
        ctx.out.sys_to_actor(me, "wod", format!("They look {word}."));
    } else {
        ctx.out.sys_to_actor(me, "wod", "They look comfortable enough.".to_string());
    }
    if own && !p.sources.is_empty() {
        let words: Vec<String> = p.sources.iter().map(SupportSource::label).collect();
        ctx.out.sys_to_actor(me, "wod", format!("  sources: {}", words.join(", ")));
    }
    if own && let Some(word) = housing_word(ctx, target) {
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

fn wod_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !ctx.switches.has("economy") {
        return Err(CmdError::msg("Usage: @wod/economy [migrate]"));
    }
    require_staff(ctx)?;
    let Some(cfg) = config(ctx) else {
        return Err(CmdError::msg("The WoD economy is not loaded."));
    };
    if ctx.args.raw().trim().eq_ignore_ascii_case("migrate") {
        let report = crate::economy::migrate::run(ctx, &cfg);
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
        let mut dots: Vec<String> =
            cfg.income.iter().map(|(d, n)| format!("{d}={n}")).collect();
        if dots.is_empty() {
            dots.push("(empty — no income lines are opened)".to_string());
        }
        ctx.out.sys_to_actor(me, "wod", format!("  income (major)     {}", dots.join(" ")));
        ctx.out.sys_to_actor(
            me,
            "wod",
            format!("  starting funds     {} (major)", cfg.starting_funds_major),
        );
        ctx.out.sys_to_actor(
            me,
            "wod",
            "  Every income line is bounded by [economy].mint_obligation_cap_major, re-checked \
             at every settlement."
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
                if let Some(c) = p.career_mut() {
                    c.employer = principal;
                }
                changed.push(format!("employer={v}"));
            }
            "front" => {
                let front = parse_front(v)?;
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
    // A front that does not resolve is still recorded (a staffer may be setting it up
    // ahead of the faction), and it is said out loud rather than silently inert.
    Ok(())
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
pub fn register(reg: &mut Registrar<'_>) -> Result<(), mush_core::ModError> {
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
                   domain=<org>  --  Record why you can afford your life.\n\
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
                   flip. It lists what it did. abstract->exact opens zero-balance accounts \
                   (nothing is minted); exact->abstract stops the income lines and hides the \
                   wallets, leaving every balance untouched; off stops evaluating and deletes \
                   nothing."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(wod_cmd),
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
