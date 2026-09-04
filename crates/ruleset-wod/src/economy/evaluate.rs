//! The period evaluation (08d §5.2), run as a `Coarse` tick on the configured
//! cadence clock.
//!
//! ## A pull, not a subscription
//!
//! The same shape E5b's assessment tick uses, for the same reason: there is no
//! subscription seam on a cadence, and adding one would put a ruleset callback inside
//! the economy's money tick. So the evaluation keeps a per-character cursor and works
//! every character whose cadence `position` has moved past it. A missed fire loses
//! nothing (the next one sees the same gap) and a duplicate fire does nothing (the
//! position is the idempotency key).
//!
//! ## What one fire may do (look-back fold, D-LBK-2)
//!
//! - **visit** at most [`MAX_VISITED_PER_FIRE`] characters — touch at all, due or
//!   not — and **work** at most [`MAX_WORKED_PER_FIRE`] of them, both enforced
//!   independently, from a **rotation cursor** that advances past the last *visited*
//!   id (skipped ones included), so every fire's scan is bounded and a world with
//!   more characters than either budget still reaches every one of them;
//! - **read the small `eval:{obj}` cursor before cloning a profile**, so an
//!   all-skipped fire clones nothing;
//! - **skip the scan entirely between periods**: the tick records the cadence
//!   position and the population version at the end of its last complete sweep that
//!   found nothing due, and a fire skips its scan only while both are unchanged —
//!   explicit state, never a guess. `put_profile` bumps the version for a new
//!   profile; a cadence advance moves the position;
//! - **one period per character however far the clock jumped** — a cadence that
//!   advanced forty periods over a weekend of downtime evaluates once. That is E4's
//!   "a backlog is never charged" in its career form: catch-up is computed from the
//!   cadence position, never from a tick count, and a restart must not spend three
//!   periods of strain while the player was asleep.
//!
//! ## What one period does, in order
//!
//! 1. **apply the pending stance**, so a flip lands on the boundary and not before
//!    (D-E5-6);
//! 2. **re-read `backgrounds:Resources` from the sheet** and derive the level as
//!    `resources ± stance`, clamped `0..=5`. The sheet is **read** here and written
//!    nowhere (D-E5-3);
//! 3. **apply the stance's strain delta**, clamped `0..=STRAIN_MAX`;
//! 4. **expire a lapsed prompt** under its own default (never a failure);
//! 5. **submit a `SupportPlan` to housing for every live abstract tenancy the
//!    character holds — every period**, because a plan older than
//!    `PLAN_MAX_AGE_PERIODS` is *absent* (E5a fold S4);
//! 6. **open at most one prompt**: housing pressure first (it preempts), then the
//!    stance's own row, under the attention budget and the per-faction pool;
//! 7. **emit the front-attributed canon evidence** an `Exploit` period owes;
//! 8. **maintain the income obligation**, in exact mode only.
//!
//! ## Exhaustive by construction
//!
//! Every match here is exhaustive and there is no `unwrap`, no `expect` and no
//! panicking index; the one `?` is an `Option` `?` in [`behind_tenancy`] (a missing
//! housing store reads as "nothing is behind"), which cannot panic. A tick handler
//! runs in the world task, so a panic here takes the world down (the discipline
//! `mush-economy`'s sweep states, `mush-housing`'s tick inherits, and this file
//! inherits from both).

use mush_canon::{CanonDraft, CanonScope};
use mush_core::{Ctx, ObjId, SceneId};
use mush_housing::HousingStore;
use smol_str::SmolStr;

use crate::economy::config::{FAUCET_INCOME, PersonalMode, WodEconomyConfig};
use crate::economy::prompt::{self, Declined};
use crate::economy::record::{
    COAST_OPPORTUNITY_PERIODS, PromptDefault, PromptKind, PromptShape, STRAIN_MAX, SpendingProfile,
    Stance, derive_level,
};
use crate::economy::store::{
    EvalState, EvalStats, IdleMark, IdleScan, MAX_VISITED_PER_FIRE, MAX_WORKED_PER_FIRE,
    WodEconomyStore, persist_meta, put_eval, put_profile,
};

/// The canon `kind` an `Exploit` period's evidence carries (§5.3).
pub const KIND_EXPOSURE: &str = "wod.career.exposure";
/// The canon `kind` housing pressure carries when it is chronicled (§5.3).
pub const KIND_HOUSING_PRESSURE: &str = "wod.housing.pressure";

/// One fire of the evaluation tick.
pub fn tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    let mut stats = EvalStats::default();
    let Some((cfg, cadence, cursor, ids, version)) = ctx.w.module_state::<WodEconomyStore>().map(|s| {
        (s.config().clone(), s.cadence(), s.cursor, s.profiled(), s.population_version)
    }) else {
        return;
    };
    if !cfg.personal_mode.is_on() || ids.is_empty() {
        if let Some(store) = ctx.w.module_state_mut::<WodEconomyStore>() {
            store.scan = None;
            store.last_fire = stats;
        }
        return;
    }
    // A clock that does not resolve makes nothing happen — the economy's own rule,
    // and E5b's. It is loud once per fire rather than silent, because a campaign that
    // has stopped being evaluated is a staff question.
    let Some(cadence) = cadence else {
        tracing::error!(
            cadence = %cfg.cadence,
            "wod.economy: the configured cadence clock does not resolve; nothing is being \
             evaluated (@econ/cadence create)"
        );
        return;
    };
    let Some(now) = mush_housing::cadence_now(ctx.w, cadence) else {
        tracing::error!(cadence = cadence.0, "wod.economy: the cadence record has gone away");
        return;
    };
    // The short-circuit (D-LBK-2): nothing was due at the end of the last complete
    // sweep, nobody has joined the population since, and the clock has not moved.
    // Then nothing is due now, and the scan is skipped.
    let mut scan = {
        let Some(store) = ctx.w.module_state_mut::<WodEconomyStore>() else { return };
        if store.idle.as_ref().is_some_and(|m| m.position == now && m.version == version) {
            stats.short_circuited = true;
            store.last_fire = stats;
            return;
        }
        store.idle = None;
        match store.scan.take() {
            Some(s) if s.position == now && s.version == version && s.population == ids.len() => s,
            _ => IdleScan { position: now, version, visited: 0, population: ids.len(), clean: true },
        }
    };

    // The rotation: start at the first id at or past the cursor, wrap once.
    let start = ids.iter().position(|id| id.0 >= cursor).unwrap_or(0);
    let order: Vec<ObjId> = ids[start..].iter().chain(ids[..start].iter()).copied().collect();

    let mut last = None;
    for who in order {
        if stats.visited >= MAX_VISITED_PER_FIRE || stats.worked >= MAX_WORKED_PER_FIRE {
            break;
        }
        stats.visited += 1;
        last = Some(who);
        if step(ctx, &cfg, who, now, &mut stats) {
            stats.worked += 1;
            scan.clean = false;
        }
    }
    scan.visited = scan.visited.saturating_add(stats.visited);
    let Some(store) = ctx.w.module_state_mut::<WodEconomyStore>() else { return };
    // Resume **past** the last character this fire visited — skipped ones included.
    if let Some(last) = last {
        store.cursor = last.0.saturating_add(1);
    }
    if scan.visited >= scan.population {
        if scan.clean && store.population_version == version {
            store.idle = Some(IdleMark { position: now, version });
        }
        store.scan = None;
    } else {
        store.scan = Some(scan);
    }
    store.last_fire = stats;
    if last.is_some() {
        persist_meta(ctx);
    }
}

/// Evaluate one character for one period. Returns whether a period was spent.
fn step(ctx: &mut Ctx<'_>, cfg: &WodEconomyConfig, who: ObjId, now: i64, stats: &mut EvalStats) -> bool {
    // **The small cursor document first, the profile only once it is due** (D-LBK-2).
    let state = ctx.w.module_state::<WodEconomyStore>().map_or_else(EvalState::default, |s| {
        s.eval_state(who)
    });
    // A profile created between two fires has never been evaluated, and a zeroed
    // cursor on a clock at period 900 would read as "900 periods are owed".
    if !state.seeded {
        put_eval(ctx, who, EvalState { position: now, period: 0, seeded: true });
        return false;
    }
    if now <= state.position {
        return false;
    }
    let Some(mut profile) =
        ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(who).cloned())
    else {
        return false;
    };
    stats.cloned += 1;
    // A character who has left the world is not evaluated and is not deleted: their
    // profile is the record of what they had, and D-E5-9's "off never deletes a
    // profile" is the same rule one object down.
    //
    // Liveness is existence **plus** not-`GOING` (E5c fold T4): core keeps a
    // destroyed object resolvable until `World::sweep` tombstones it, so an existence
    // test alone lets a destroyed character gain strain and leave a
    // `wod.career.exposure` record naming them. Every neighbour defines it this way
    // (`mush_housing::alive`, `mush-economy`'s custody checks); this is that
    // predicate, inline, on the one path that needs it.
    if !ctx.w.get(who).is_some_and(|o| !o.flags.contains(mush_core::Flags::GOING)) {
        return false;
    }
    let period = state.period.saturating_add(1);

    // 1 — the pending stance lands on the boundary.
    if let Some(career) = profile.career_mut()
        && let Some(next) = career.pending_stance.take()
    {
        career.stance = next;
        profile.note(now, format!("stance now {}", next.label()));
    }
    // `careers = false` means careers, stances, opportunities and exposure are **not
    // evaluated** — the config field's own doc comment, and the switch has to mean it
    // (E5c fold D2). Taking the stance as `Coast` is what makes the whole row inert:
    // no `level_delta` buying a support tier, no `strain_delta`, no offer, no
    // exposure. `+career/stance` refuses on such a server, so `pending_stance` can
    // never be set there either.
    let stance = if cfg.careers { profile.stance() } else { Stance::Coast };

    // 2 — re-read the sheet. Read only: the economy never writes a dot (D-E5-3).
    profile.resources = read_resources(ctx, who);
    profile.level = derive_level(profile.resources, stance.level_delta());

    // 3 — strain.
    profile.apply_strain(if profile.career().is_some() {
        stance.strain_delta()
    } else {
        // No career, no stance: §5.2's plain decay, and only while nothing is behind.
        if behind_tenancy(ctx, who).is_some() { 0 } else { -1 }
    });

    // 4 — a lapsed prompt, under its own default.
    let lapsed = prompt::expire(&mut profile, period, now);

    profile.revision = profile.revision.saturating_add(1);
    put_profile(ctx, profile.clone());
    put_eval(ctx, who, EvalState { position: now, period, seeded: true });

    // 5 — the plan, every period, for every live abstract tenancy.
    submit_plans(ctx, &profile);

    // 6 — at most one prompt. Housing pressure first: it preempts, and the reverse
    // is never true (D-E5-8).
    offer_prompts(ctx, cfg, &profile, stance, period, now, lapsed);

    // 7 — the exposure an `Exploit` period owes.
    if cfg.careers && stance.emits_exposure() {
        emit_exposure(ctx, who, period, now);
    }

    // 8 — exact mode's income obligation.
    if cfg.personal_mode == PersonalMode::Exact {
        maintain_income(ctx, cfg, who);
    }
    true
}

/// `backgrounds:Resources`, case-insensitively, clamped into the band.
///
/// **The one sheet read in the whole layer.** `backgrounds` is a Map field, so
/// `SheetDoc::dots` does not apply to it; this is the same case-insensitive lookup
/// the splats use (`splats/vampire.rs`'s `background_dots`).
fn read_resources(ctx: &Ctx<'_>, who: ObjId) -> u8 {
    crate::economy::sheet_resources(ctx.w, who).unwrap_or(0)
}

/// The first tenancy of `who`'s that is behind, if any.
///
/// **The prompt opens at `Delinquent`, and the `Grace` rung is housing's own notice**
/// (E5c fold T5, documented rather than changed). `TenancyStatus::is_behind()` is
/// `Delinquent | EvictionPending` and deliberately excludes `Grace`, so the player's
/// first *asked* moment is one rung after housing's first told moment. That is the
/// accepted split: 08d §5.3's "housing's own notices are notices, not prompts", and
/// §11 default 1 fixes `GRACE_PERIODS` at 1 as a cushion meant to resolve silently.
/// The tenant is not left in the dark in the meantime — they receive housing's
/// private Grace notice.
fn behind_tenancy(ctx: &Ctx<'_>, who: ObjId) -> Option<mush_housing::TenancyId> {
    ctx.w
        .module_state::<HousingStore>()?
        .live_tenancies_of_tenant(who)
        .find(|t| t.status.is_behind())
        .map(|t| t.id)
}

/// Submit this period's [`mush_housing::SupportPlan`] for every live tenancy `who`
/// holds (§5.2, answer 17).
///
/// **Every period**, because a plan older than `PLAN_MAX_AGE_PERIODS` is *absent*,
/// not asserted (E5a fold S4): an evaluator that stops running has to let
/// `UNKNOWN_GRACE` fire rather than propping a lease up on an answer nobody has
/// restated. Housing stamps both `asserted_at` and `asserted_period` itself, and a
/// restatement that changes nothing writes nothing.
fn submit_plans(ctx: &mut Ctx<'_>, profile: &SpendingProfile) {
    let ids: Vec<mush_housing::TenancyId> = ctx
        .w
        .module_state::<HousingStore>()
        .map(|s| s.live_tenancies_of_tenant(profile.obj).map(|t| t.id).collect())
        .unwrap_or_default();
    let level = mush_housing::SupportLevel::new(profile.level);
    let sources = profile.housing_sources();
    for id in ids {
        match mush_housing::submit_support_plan(ctx, id, profile.obj, level, sources.clone()) {
            Ok(()) => {}
            // An **exact** tenancy is funded by its lease; a plan does not apply to
            // it, and that is the expected answer rather than a fault.
            Err(mush_housing::HousingError::WrongFunding) => {}
            Err(error) => {
                tracing::warn!(
                    tenancy = id.0, who = profile.obj.0, %error,
                    "wod.economy: housing refused a support plan"
                );
            }
        }
    }
}

/// Offer this period's one prompt, in preemption order (§5.3).
fn offer_prompts(
    ctx: &mut Ctx<'_>,
    cfg: &WodEconomyConfig,
    profile: &SpendingProfile,
    stance: Stance,
    period: u64,
    now: i64,
    lapsed: Option<PromptDefault>,
) {
    // **One period of quiet for every prompt kind** (look-back fold, E5c-C96; 08g
    // §12.1 item 3). A prompt that lapsed **this** period does not immediately get
    // replaced — not by an opportunity, and not by housing pressure or strain relief
    // either: the point of the budget is quiet, and answering nothing must not
    // summon the next question in the same breath. A standing condition re-asks
    // next period; the housing ladder pages regardless, on its own schedule.
    if lapsed.is_some() {
        return;
    }
    // Housing pressure: a home that is behind asks, whatever else is open, and
    // whatever the character's engagement is — it is not an opportunity.
    if cfg.housing && let Some(tenancy) = behind_tenancy(ctx, profile.obj) {
        let already = profile
            .prompt
            .as_ref()
            .is_some_and(|p| matches!(p.kind, PromptKind::HousingPressure { .. }));
        if !already {
            let _ = prompt::offer(
                ctx,
                profile.obj,
                PromptKind::HousingPressure { tenancy },
                prompt::housing_options(),
                // The window stays with the ST when nobody answers — the one case
                // where the conservative default is "a person is still holding it".
                PromptDefault::PendingWindow,
                now,
                period,
            );
            return;
        }
    }
    // Strain at the ceiling asks once, and is not front-attributed.
    if profile.strain >= STRAIN_MAX && profile.prompt.is_none() {
        let _ = prompt::offer(
            ctx,
            profile.obj,
            PromptKind::StrainRelief,
            prompt::relief_options(),
            PromptDefault::StatusQuo,
            now,
            period,
        );
        return;
    }
    if !cfg.careers {
        return;
    }
    let Some(career) = profile.career() else { return };
    // Ambient is the quiet default: the career still supports the lease and still
    // carries strain; it simply never asks (§5.2's `engagement`).
    if profile.engagement != crate::economy::config::Engagement::Engaged {
        return;
    }
    let Some(shape) = stance.offers() else { return };
    // `Coast` offers rarely (§5.2's table + §9's `COAST_OPPORTUNITY_PERIODS`).
    //
    // A stamp from a **future** ordinal is stale, not fresh — the same guard
    // `Prompt::expired` takes (E5c fold T3). Without it a re-seeded `eval:{obj}`
    // cursor would silence `Coast` for six periods it never spent, because the
    // saturating subtraction reads 0 for every ordinal below the stamp.
    let stale_stamp = profile.last_opportunity_period > period;
    if stance == Stance::Coast
        && !stale_stamp
        && period.saturating_sub(profile.last_opportunity_period) < COAST_OPPORTUNITY_PERIODS
    {
        return;
    }
    // Every opportunity cites a specific front (owner answer 20). A career with no
    // front generates nothing at all — quiet by construction, not by accident.
    let Some(front) = career.front else { return };
    let kind = match shape {
        PromptShape::Opportunity => PromptKind::Opportunity { front },
        PromptShape::Advancement => PromptKind::Advancement { front },
    };
    let options = match shape {
        PromptShape::Opportunity => prompt::opportunity_options(),
        PromptShape::Advancement => prompt::advancement_options(),
    };
    match prompt::offer(ctx, profile.obj, kind, options, PromptDefault::StatusQuo, now, period) {
        Ok(_) => {
            // Only a prompt that actually opened moves the `Coast` rate limit — a
            // declined offer must not cost the next six periods' worth of chances.
            if let Some(mut p) =
                ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(profile.obj).cloned())
            {
                p.last_opportunity_period = period;
                put_profile(ctx, p);
            }
        }
        Err(Declined::Budget | Declined::Pool | Declined::OrphanedFront) => {}
        Err(Declined::Exhausted) => {
            tracing::error!(
                who = profile.obj.0,
                "wod.economy: the prompt-id allocator is exhausted; no prompt was opened"
            );
        }
    }
}

/// Commit one front-attributed canon evidence event for an `Exploit` period (§5.3).
///
/// These are **canon records, not settlement attestors**: E4c's fold requires an
/// attestor kind to be a producer-declared outcome registered through
/// `with_canon_outcome_kinds`, and `wod.career.exposure` is deliberately not one.
/// It is evidence a Storyteller reads, and a contract gated on it would be one a
/// player could seal for themselves.
fn emit_exposure(ctx: &mut Ctx<'_>, who: ObjId, period: u64, now: i64) {
    let Some(front) = ctx
        .w
        .module_state::<WodEconomyStore>()
        .and_then(|s| s.profile(who).and_then(|p| p.career().and_then(|c| c.front)))
    else {
        return;
    };
    // An orphaned front **stops generating** (§5.3) — including this.
    if !prompt::front_resolves(ctx, &front) {
        return;
    }
    let name = ctx
        .w
        .get(who)
        .map_or_else(|| format!("#{}", who.0), |o| {
            mush_core::output::primary_name(&o.name).to_string()
        });
    let ic = ctx.world_ic_now();
    let ic_time = ctx.w.game_now_unix();
    let detail = serde_json::json!({
        "faction": front.faction.0,
        "front": front.front.0,
        "operation": front.operation,
        "period": period,
        "cadence_position": now,
        // The whole K4a stamp, generation and all, so nothing downstream can compare
        // it across a rate change by accident.
        "world_ic": ic,
    });
    mush_canon::commit(ctx, CanonDraft {
        kind: SmolStr::new_static(KIND_EXPOSURE),
        summary: format!("{name} left a trace working an angle."),
        entities: vec![who],
        // Evidence, not a headline: it is about the character and whoever can read
        // their file, not the whole chronicle.
        scope: CanonScope::Restricted,
        ic_time,
        source: front.source(),
        detail,
    });
}

/// Keep the exact-mode income obligation matching the character's **Resources dot**
/// (§5.2, D-E5-6).
///
/// **The row is keyed on the sheet reading, never on the derived level** (E5c fold
/// M1). `profile.level` is `resources ± stance`, and a stance is an ungated player
/// verb: keying the mint off it would let `+career/stance exploit` move a character
/// one whole income row up on the same `ObligationId`, and `conceal` move it down or
/// cancel it outright. D-E5-6 is explicit that a stance alters *pressure and
/// opportunity* and never income, and §5.1's table is "one amount per **dot**". The
/// stance loses nothing the design promises: `profile.level` is still the number
/// `submit_plans` hands housing, which is where §5.2's table delivers Exploit's
/// "temporary capacity".
///
/// Every line is a **faucet-funded obligation** created through
/// `create_system_obligation` under `system:wod.income` — never `@obligation/create`,
/// which refuses a `System` debit outright — and every per-period amount is bounded
/// by `[economy].mint_obligation_cap_major` (with the `wod.usd` override),
/// re-checked at every settlement (08c §4.8 M1).
fn maintain_income(ctx: &mut Ctx<'_>, cfg: &WodEconomyConfig, who: ObjId) {
    let Some(profile) = ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(who).cloned())
    else {
        return;
    };
    let Some(instrument) =
        ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.instrument().cloned())
    else {
        return;
    };
    let Some(credit) = profile.account else { return };
    let want = cfg
        .income_major(profile.resources)
        .and_then(|major| crate::economy::boot::major_to_amount(ctx, &instrument, major));

    // A hand cancel is a **decision**, and a decision is not re-derived away — E5b's
    // own rule for `Withdrawn`, one record class over (E5c fold M4). The mark is
    // dropped the moment the dot it was taken at stops being the dot on the sheet.
    if let Some(dot) = profile.income_cancelled_at
        && dot != profile.resources
    {
        let mut p = profile.clone();
        p.income_cancelled_at = None;
        p.revision = p.revision.saturating_add(1);
        put_profile(ctx, p);
    }
    let stopped_by_hand = profile.income_cancelled_at == Some(profile.resources);

    // A live obligation this profile already owns.
    let live = profile.income.and_then(|id| {
        ctx.w.module_state::<mush_economy::EconomyStore>().and_then(|s| {
            s.snapshot().obligation(id).map(|o| (id, o.amount, o.status.kind()))
        })
    });
    // A pointer at a record that is not there any more: the pointer is a cache, and a
    // cache with nothing behind it is cleared (E5c fold M3).
    if profile.income.is_some() && live.is_none() {
        clear_income(ctx, who);
    }
    let Some(faucet) = crate::economy::boot::principal(FAUCET_INCOME) else { return };
    match (live, want) {
        // Nothing owed and nothing running: the common case.
        (None, None) => {}
        // The level now earns nothing: stop the standing mint.
        //
        // **The pointer is cleared only when the mint is actually stopped** (E5c fold
        // M3). Clearing it after a failed cancel — `EconomyFrozen` from the boot
        // integrity latch is reachable on a period the clock already advanced — would
        // strand a live faucet-funded line nothing names, and the next period's
        // `open_income` would credit the same wallet a second time.
        (Some((id, _, kind)), None) => {
            if matches!(
                kind,
                mush_economy::ObligationStatusKind::Cancelled
                    | mush_economy::ObligationStatusKind::Satisfied
            ) {
                clear_income(ctx, who);
            } else {
                match mush_economy::set_system_obligation(ctx, &faucet, id, None) {
                    Ok(()) => clear_income(ctx, who),
                    Err(error) => tracing::warn!(
                        obligation = id.0, who = who.0, %error,
                        "wod.economy: could not stop an income line; the profile keeps the \
                         pointer and the next evaluation retries"
                    ),
                }
            }
        }
        // Running at the wrong rate: re-rate it in place. The schedule, the grace
        // policy and the counters are untouched, so this is not a re-anchor.
        (Some((id, have, kind)), Some(amount)) => {
            if matches!(
                kind,
                mush_economy::ObligationStatusKind::Cancelled
                    | mush_economy::ObligationStatusKind::Satisfied
            ) {
                // Somebody cancelled it. Record the decision and leave it alone: a
                // staffer — or the creditor themself — stopping one character's
                // stipend must not come back as a new obligation id with nothing
                // said (E5c fold M4).
                mark_income_cancelled(ctx, who, id);
            } else if have != amount
                && let Err(error) =
                    mush_economy::set_system_obligation(ctx, &faucet, id, Some(amount))
            {
                tracing::warn!(obligation = id.0, %error, "wod.economy: could not re-rate an income line");
            }
        }
        // Owed and not running: open it — unless a person stopped this one.
        (None, Some(amount)) => {
            if !stopped_by_hand {
                open_income(ctx, &faucet, who, &instrument, credit, amount);
            }
        }
    }
}

/// Forget the income obligation on a profile (it was cancelled or has gone away).
fn clear_income(ctx: &mut Ctx<'_>, who: ObjId) {
    let Some(mut p) = ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(who).cloned())
    else {
        return;
    };
    if p.income.is_none() {
        return;
    }
    p.income = None;
    p.revision = p.revision.saturating_add(1);
    put_profile(ctx, p);
}

/// Record that a person cancelled this character's income line, and stop naming it
/// (E5c fold M4).
///
/// The note is the whole point: the layer says what it saw once, and then does
/// nothing about it ever again until the dot moves, `@wod/economy migrate` runs, or
/// staff open the next one deliberately.
fn mark_income_cancelled(ctx: &mut Ctx<'_>, who: ObjId, id: mush_economy::ObligationId) {
    let Some(mut p) = ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(who).cloned())
    else {
        return;
    };
    if p.income.is_none() && p.income_cancelled_at == Some(p.resources) {
        return;
    }
    p.income = None;
    p.income_cancelled_at = Some(p.resources);
    p.note(0, format!("income line #{} was cancelled; none is opened in its place", id.0));
    p.revision = p.revision.saturating_add(1);
    put_profile(ctx, p);
}

/// Open this character's income line under the faucet (§5.1).
///
/// **Never a second one.** A live `wod.income` obligation already crediting `credit`
/// is adopted rather than duplicated (E5c fold M3): the record carries its own
/// `cause`, `minted_by` and `credit_account`, so a profile whose pointer was lost —
/// or cleared by a failed cancel — finds its own mint again instead of opening a
/// parallel faucet line into the same wallet.
fn open_income(
    ctx: &mut Ctx<'_>,
    faucet: &mush_org::SystemPrincipalId,
    who: ObjId,
    instrument: &mush_economy::InstrumentId,
    credit: mush_economy::AccountId,
    amount: mush_economy::Amount,
) {
    if let Some((existing, _)) = crate::economy::boot::live_income_lines(ctx)
        .into_iter()
        .find(|(_, account)| *account == credit)
    {
        if let Some(mut p) =
            ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(who).cloned())
        {
            p.income = Some(existing);
            p.revision = p.revision.saturating_add(1);
            p.note(0, format!("income line #{} adopted (it was already running)", existing.0));
            put_profile(ctx, p);
        }
        tracing::warn!(
            obligation = existing.0, who = who.0,
            "wod.economy: a live income line already credits this wallet; adopted it rather than \
             opening a second"
        );
        return;
    }
    let Some(debit) = crate::economy::boot::system_account(ctx, FAUCET_INCOME, instrument) else {
        return;
    };
    let Some(cadence) = ctx.w.module_state::<WodEconomyStore>().and_then(WodEconomyStore::cadence)
    else {
        return;
    };
    let Some(next_due) = mush_housing::cadence_now(ctx.w, cadence) else { return };
    let spec = mush_economy::ObligationSpec {
        debit_account: debit,
        credit_account: credit,
        amount,
        // On the campaign clock, one period at a time. `next_due` and `cadence` are
        // counted in cadence **periods**, not seconds (E4's `EconomicSchedule`).
        schedule: mush_economy::EconomicSchedule::new(
            mush_economy::EconomicClock::Cadence(cadence),
            next_due.saturating_add(1),
            Some(1),
            mush_economy::DEFAULT_CATCH_UP,
        ),
        grace: mush_economy::GracePolicy::new(mush_economy::DEFAULT_GRACE_PERIODS),
        notify_on_success: false,
        cause: mush_economy::ObligationCause::Ruleset {
            namespace: SmolStr::new_static("wod.income"),
        },
    };
    match mush_economy::create_system_obligation(ctx, faucet, spec) {
        Ok(id) => {
            if let Some(mut p) =
                ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(who).cloned())
            {
                p.income = Some(id);
                p.revision = p.revision.saturating_add(1);
                put_profile(ctx, p);
            }
        }
        Err(error) => {
            tracing::warn!(
                who = who.0, %error,
                "wod.economy: could not open an income line (the [economy] mint cap bounds every \
                 per-period amount)"
            );
        }
    }
}
