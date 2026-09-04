//! The attention budget, the preemption rule, the per-faction pool, and
//! non-response (08d §5.3, owner answers 18–20, D-E5-8).
//!
//! Four rules, each of which exists to stop a different kind of tedium:
//!
//! 1. **One unresolved prompt per character.** A second one is *dropped*, never
//!    queued — a queue is how "at most one" becomes "a backlog of one at a time".
//! 2. **Housing pressure preempts, and is never preempted.** Somebody's home
//!    outranks a job offer in both directions.
//! 3. **A bounded per-faction pool of currently open operations.** When a faction
//!    already has [`MAX_OFFSCREEN_OPS_PER_FACTION`] operations open across the whole
//!    world, it generates no more until one resolves or expires. The pool is
//!    *derived* from the live prompts ([`WodEconomyStore::open_ops`]), so the
//!    decrement is automatic and there is no counter to drift.
//! 4. **Non-response is the status quo.** Expiry applies the prompt's `default` and
//!    **never** adds strain, lowers support, moves money, or reads as a failure.

use mush_core::{Ctx, ObjId};

use crate::economy::record::{
    MAX_OFFSCREEN_OPS_PER_FACTION, PROMPT_EXPIRY_PERIODS, Prompt, PromptDefault, PromptKind,
    PromptOption, PromptOutcome, SpendingProfile,
};
use crate::economy::store::{WodEconomyStore, persist_meta, put_profile};

/// Why an offered prompt was not opened. Every arm is a *quiet* outcome — nothing
/// here is an error a player sees.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Declined {
    /// The character already has an unresolved prompt this one may not displace.
    Budget,
    /// The faction's off-screen pool is full.
    Pool,
    /// The front no longer resolves (§5.3: an orphaned front stops generating).
    OrphanedFront,
    /// The id allocator is at the ceiling.
    Exhausted,
}

/// Whether `front` still resolves against the live faction state (§5.3).
///
/// An **orphaned** front — a `FrontId` that no longer resolves after a K or faction
/// change — renders as its raw id and **stops generating**. It never fails a read and
/// it never panics: the faction module simply may not be loaded at all, which is the
/// same answer as "that front is gone".
#[must_use]
pub fn front_resolves(ctx: &Ctx<'_>, front: &crate::economy::record::FrontRef) -> bool {
    let faction_ok = ctx
        .w
        .module_state::<mush_faction::FactionState>()
        .is_some_and(|s| s.factions.contains_key(&front.faction));
    let front_ok = ctx
        .w
        .module_state::<mush_faction::SimState>()
        .is_some_and(|s| s.fronts.get(&front.front).is_some_and(|f| f.faction == front.faction));
    faction_ok && front_ok
}

/// Render a front for a player-facing line, tolerating an orphan (§5.3).
#[must_use]
pub fn render_front(ctx: &Ctx<'_>, front: &crate::economy::record::FrontRef) -> String {
    if !front_resolves(ctx, front) {
        return format!("{} (orphaned front)", front.raw());
    }
    let faction = ctx
        .w
        .module_state::<mush_faction::FactionState>()
        .and_then(|s| s.factions.get(&front.faction).map(|f| f.name.to_string()))
        .unwrap_or_else(|| format!("#{}", front.faction.0));
    let title = ctx
        .w
        .module_state::<mush_faction::SimState>()
        .and_then(|s| s.fronts.get(&front.front).map(|f| f.title.clone()))
        .unwrap_or_else(|| format!("#{}", front.front.0));
    match front.operation {
        Some(op) => format!("{faction}: {title} (op #{op})"),
        None => format!("{faction}: {title}"),
    }
}

/// Offer `kind` to `who`, subject to all four rules.
///
/// Returns `Ok(id)` when a prompt was opened, `Err(reason)` when it was declined.
/// **Declining is silent by design**: the whole point of the budget is that a
/// character is not told about the things they were spared.
pub fn offer(
    ctx: &mut Ctx<'_>,
    who: ObjId,
    kind: PromptKind,
    options: Vec<PromptOption>,
    default: PromptDefault,
    now: i64,
    period: u64,
) -> Result<u64, Declined> {
    // Rule 3, first, because a full pool means the *faction* generates nothing and
    // asking the character's budget about it would be beside the point.
    if let Some(front) = kind.front() {
        if !front_resolves(ctx, &front) {
            return Err(Declined::OrphanedFront);
        }
        let open = ctx
            .w
            .module_state::<WodEconomyStore>()
            .map_or(0, |s| s.open_ops_for(front.faction));
        if open >= MAX_OFFSCREEN_OPS_PER_FACTION {
            return Err(Declined::Pool);
        }
    }
    let Some(mut profile) = ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(who).cloned())
    else {
        return Err(Declined::Budget);
    };
    // Rules 1 + 2 — the attention budget and preemption.
    let displaced = match (&profile.prompt, &kind) {
        (None, _) => None,
        // Housing pressure displaces the two front-attributed kinds…
        (Some(open), PromptKind::HousingPressure { .. })
            if open.kind.displaceable_by_housing() =>
        {
            Some(open.clone())
        }
        // …and nothing displaces anything else. A second prompt is dropped.
        (Some(_), _) => return Err(Declined::Budget),
    };
    let Some(id) = ctx.w.module_state_mut::<WodEconomyStore>().and_then(WodEconomyStore::alloc_prompt)
    else {
        return Err(Declined::Exhausted);
    };
    if let Some(old) = displaced {
        profile.note(now, format!("prompt #{} dropped for housing pressure", old.id));
    }
    profile.prompt = Some(Prompt {
        id,
        kind,
        options,
        default,
        opened_at: now,
        opened_period: period,
        opened_ic: Some(ctx.world_ic_now()),
        expires_after_periods: PROMPT_EXPIRY_PERIODS,
    });
    profile.revision = profile.revision.saturating_add(1);
    put_profile(ctx, profile);
    persist_meta(ctx);
    announce(ctx, who);
    Ok(id)
}

/// Tell a character they have something waiting. One line — the prompt itself is a
/// pull (`+prompt`), per 08d §1.1's "routine visibility is pull-only".
fn announce(ctx: &mut Ctx<'_>, who: ObjId) {
    let Some(p) = ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(who).cloned())
    else {
        return;
    };
    let Some(prompt) = &p.prompt else { return };
    let what = prompt.kind.label();
    ctx.out.info(who, format!("Something has come up ({what}). Type +prompt to look at it."));
}

/// Apply a chosen option (`+prompt <option>`).
///
/// # Errors
///
/// A message for the player when there is no prompt or no such option.
pub fn choose(ctx: &mut Ctx<'_>, who: ObjId, key: &str, now: i64) -> Result<String, String> {
    let Some(mut profile) = ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(who).cloned())
    else {
        return Err("You have nothing waiting.".to_string());
    };
    let Some(prompt) = profile.prompt.clone() else {
        return Err("You have nothing waiting.".to_string());
    };
    let Some(option) = prompt.option(key).cloned() else {
        let keys: Vec<&str> = prompt.options.iter().map(|o| o.key.as_str()).collect();
        return Err(format!("Choose one of: {}.", keys.join(", ")));
    };
    match option.outcome {
        PromptOutcome::StatusQuo => {}
        PromptOutcome::Strain(n) => profile.apply_strain(i16::from(n)),
        PromptOutcome::Relief(n) => profile.apply_strain(-i16::from(n)),
    }
    profile.note(now, format!("prompt #{} answered: {}", prompt.id, option.key));
    profile.prompt = None;
    profile.revision = profile.revision.saturating_add(1);
    put_profile(ctx, profile);
    Ok(option.label.to_string())
}

/// Expire `who`'s prompt if its life is up, applying its `default` (§5.3).
///
/// Returns whether anything expired. The profile is **handed back by value** rather
/// than written here, because the evaluation that calls this is already holding one
/// and writing it twice in a period would double the revision for no reason.
pub fn expire(profile: &mut SpendingProfile, period: u64, now: i64) -> Option<PromptDefault> {
    let prompt = profile.prompt.as_ref()?;
    if !prompt.expired(period) {
        return None;
    }
    let (id, default) = (prompt.id, prompt.default);
    // **Nothing but a note.** No strain, no support change, no money, and the wording
    // is not a failure (owner answer 19).
    profile.note(now, match default {
        PromptDefault::StatusQuo => format!("prompt #{id} lapsed; nothing changed"),
        PromptDefault::PendingWindow => format!("prompt #{id} lapsed; staff still have it"),
    });
    profile.prompt = None;
    profile.revision = profile.revision.saturating_add(1);
    Some(default)
}

/// The two standard answers to an opportunity: take it on, or let it go. Neither
/// costs support and neither costs money — a stance alters pressure, never income
/// (D-E5-6).
#[must_use]
pub fn opportunity_options() -> Vec<PromptOption> {
    vec![
        PromptOption {
            key: smol_str::SmolStr::new_static("take"),
            label: smol_str::SmolStr::new_static(
                "Take it on — more to juggle, and something to show for it.",
            ),
            outcome: PromptOutcome::Strain(1),
        },
        PromptOption {
            key: smol_str::SmolStr::new_static("pass"),
            label: smol_str::SmolStr::new_static("Let it go by. Nothing changes."),
            outcome: PromptOutcome::StatusQuo,
        },
    ]
}

/// The answers to an advancement prompt. **No automatic dot gain** (owner answer
/// 11): a permanent Resources change goes through `+xp/spend`, and this says so.
#[must_use]
pub fn advancement_options() -> Vec<PromptOption> {
    vec![
        PromptOption {
            key: smol_str::SmolStr::new_static("push"),
            label: smol_str::SmolStr::new_static(
                "Push for it — the hours cost you. Raise the trait with +xp/spend when you can.",
            ),
            outcome: PromptOutcome::Strain(1),
        },
        PromptOption {
            key: smol_str::SmolStr::new_static("steady"),
            label: smol_str::SmolStr::new_static("Stay where you are. Nothing changes."),
            outcome: PromptOutcome::StatusQuo,
        },
    ]
}

/// The answers to a housing-pressure prompt. Neither of them pays rent: money moves
/// through E4 and nowhere else, and a prompt is a question about attention.
#[must_use]
pub fn housing_options() -> Vec<PromptOption> {
    vec![
        PromptOption {
            key: smol_str::SmolStr::new_static("scrape"),
            label: smol_str::SmolStr::new_static(
                "Scrape it together somehow — it costs you to do it.",
            ),
            outcome: PromptOutcome::Strain(1),
        },
        PromptOption {
            key: smol_str::SmolStr::new_static("wait"),
            label: smol_str::SmolStr::new_static(
                "Let it ride. Nothing about where you live changes today.",
            ),
            outcome: PromptOutcome::StatusQuo,
        },
    ]
}

/// The answers to a strain-relief prompt.
#[must_use]
pub fn relief_options() -> Vec<PromptOption> {
    vec![
        PromptOption {
            key: smol_str::SmolStr::new_static("rest"),
            label: smol_str::SmolStr::new_static("Take a breath. Some of the pressure comes off."),
            outcome: PromptOutcome::Relief(2),
        },
        PromptOption {
            key: smol_str::SmolStr::new_static("push"),
            label: smol_str::SmolStr::new_static("Keep going. Nothing changes."),
            outcome: PromptOutcome::StatusQuo,
        },
    ]
}
