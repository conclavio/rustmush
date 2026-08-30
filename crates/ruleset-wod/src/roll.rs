//! `+roll` — the Storyteller-System dice command — plus the roll-log ring
//! (`+roll/last`) and the `wod.roll` panel feed.
//!
//! `+roll <pool> [vs <diff>]` resolves a trait expression (or a raw integer
//! pool) against the roller's sheet, auto-applies the wound penalty, rolls
//! through [`Owod20Pool`](mush_dice::owod20::Owod20Pool) with the server RNG,
//! and renders the classic result to the room (unless `/priv`), to a per-
//! character transient roll-log ring, and to the `wod.roll` panel with full
//! per-die records.
//!
//! Boolean modifiers are real dispatch switches: `/spec` (10s count double),
//! `/wp` (spend a temporary Willpower for +1 success), `/priv` (result to the
//! roller and staff only), `/nowound` (skip the wound penalty). Value-bearing
//! options ride the argument as inline keywords: `vs`/`diff <n>` (difficulty),
//! `mod <n>` (flat modifier), `spec <name>` (a specialty checked against the
//! ability), `ext <n>` (extended roll toward `n` successes), `job <n>` (pin the
//! result to a job), and `contested <target>=<pool>` (resolve both sides).

use std::sync::Arc;

use mush_core::{CmdError, Ctx, ObjId, Role, WorldEvent, match_object, perm};
use mush_dice::RollCtx;
use mush_dice::owod20::{Owod20Opts, Owod20Outcome, Owod20Pool, contest};
use mush_dice::pool::PoolMechanic;
use mush_proto::{NarrativeKind, OutputEvent, RichText};

use crate::WodCmd;
use crate::dice::{ResolvedPool, resolve_pool};
use crate::perception::ActPerceptibility;
use crate::state::{RollLogEntry, WodState};

/// Parsed `+roll` options (dispatch switches are read separately from `ctx`).
#[derive(Clone, Debug, Default)]
struct RollArgs {
    pool_expr: String,
    difficulty: Option<u8>,
    mod_delta: i32,
    spec_name: Option<String>,
    ext_target: Option<i32>,
    job: Option<u64>,
    contested: Option<(String, String)>,
}

/// Parse the `+roll` argument: pool expression followed by inline keyword
/// options.
fn parse_roll_args(raw: &str) -> Result<RollArgs, CmdError> {
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    let mut args = RollArgs::default();
    let mut pool = Vec::new();
    let mut i = 0;
    let is_kw = |t: &str| {
        matches!(
            t.to_ascii_lowercase().as_str(),
            "vs" | "diff" | "mod" | "spec" | "ext" | "job" | "contested"
        )
    };
    // Pool expression is everything up to the first keyword.
    while i < tokens.len() && !is_kw(tokens[i]) {
        pool.push(tokens[i]);
        i += 1;
    }
    args.pool_expr = pool.join(" ");
    while i < tokens.len() {
        let kw = tokens[i].to_ascii_lowercase();
        i += 1;
        match kw.as_str() {
            "vs" | "diff" => {
                let v = tokens
                    .get(i)
                    .ok_or_else(|| CmdError::msg("Difficulty needs a number."))?;
                args.difficulty = Some(
                    v.parse::<u8>()
                        .map_err(|_| CmdError::msg(format!("Bad difficulty: {v:?}")))?,
                );
                i += 1;
            }
            "mod" => {
                let v = tokens
                    .get(i)
                    .ok_or_else(|| CmdError::msg("mod needs a number."))?;
                args.mod_delta = v
                    .parse::<i32>()
                    .map_err(|_| CmdError::msg(format!("Bad modifier: {v:?}")))?;
                i += 1;
            }
            "spec" => {
                let v = tokens
                    .get(i)
                    .ok_or_else(|| CmdError::msg("spec needs a name."))?;
                args.spec_name = Some((*v).to_string());
                i += 1;
            }
            "ext" => {
                let v = tokens
                    .get(i)
                    .ok_or_else(|| CmdError::msg("ext needs a target."))?;
                args.ext_target = Some(
                    v.parse::<i32>()
                        .map_err(|_| CmdError::msg(format!("Bad extended target: {v:?}")))?,
                );
                i += 1;
            }
            "job" => {
                let v = tokens
                    .get(i)
                    .ok_or_else(|| CmdError::msg("job needs a number."))?;
                args.job = Some(
                    v.parse::<u64>()
                        .map_err(|_| CmdError::msg(format!("Bad job number: {v:?}")))?,
                );
                i += 1;
            }
            "contested" => {
                // Consume the rest as "target=pool".
                let rest = tokens[i..].join(" ");
                i = tokens.len();
                let (t, p) = rest
                    .split_once('=')
                    .ok_or_else(|| CmdError::msg("Usage: ... contested <target>=<pool>"))?;
                args.contested = Some((t.trim().to_string(), p.trim().to_string()));
            }
            _ => unreachable!("keyword scan"),
        }
    }
    Ok(args)
}

/// Resolve a pool against `who`'s sheet (or a raw integer if they have none),
/// using the character's active (template-selected) schema.
fn resolve_for(
    cmd: &WodCmd,
    ctx: &Ctx<'_>,
    who: ObjId,
    expr: &str,
) -> Result<ResolvedPool, String> {
    match cmd.read(ctx.w, who) {
        Some(doc) => resolve_pool(&cmd.schema_for_doc(&doc), &doc, expr),
        None => {
            let n: i32 = expr
                .trim()
                .parse()
                .map_err(|_| "That character has no sheet; roll a plain number.".to_string())?;
            Ok(ResolvedPool {
                terms: vec![crate::dice::PoolTerm {
                    label: "pool".to_string(),
                    value: n,
                }],
                total: n,
                trait_fields: Vec::new(),
            })
        }
    }
}

/// `+roll` entry point.
pub(crate) fn roll_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("last") {
        return roll_last(ctx);
    }
    let args = parse_roll_args(ctx.args.raw())?;
    if let Some((target_spec, their_expr)) = args.contested.clone() {
        return contested_roll(cmd, ctx, &args, &target_spec, &their_expr);
    }
    single_roll(cmd, ctx, &args)
}

/// Resolve difficulty, wound penalty, specialty, and willpower into options and
/// a final pool for `who` rolling `expr`. Performs the willpower deduction
/// (a sheet write) as a side effect.
#[allow(clippy::too_many_arguments)]
fn prepare(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    who: ObjId,
    args: &RollArgs,
    use_spec_switch: bool,
    use_wp: bool,
    nowound: bool,
) -> Result<Prepared, CmdError> {
    let resolved = resolve_for(cmd, ctx, who, &args.pool_expr).map_err(CmdError::msg)?;
    let doc = cmd.read(ctx.w, who);

    // Wound penalty (<= 0) and incapacitation.
    let (wound, incap) = match (&doc, nowound) {
        (Some(d), false) => {
            let track =
                crate::health::HealthTrack::from_marks(&d.track("health").unwrap_or_default());
            (
                i32::from(cmd.store_for_doc(d).wound_penalty(d, "health")),
                track.is_incapacitated(),
            )
        }
        _ => (0, false),
    };

    // Specialty (oWoD 10-again): 10s count double only when the roller
    // actually has an applicable specialty (crib). `/spec` requires *some*
    // recorded specialty on one of the pool's abilities; `spec <name>`
    // requires that specific one.
    let mut specialty = false;
    if use_spec_switch {
        let ok = doc
            .as_ref()
            .is_some_and(|d| has_any_specialty(d, &resolved.trait_fields));
        if !ok {
            return Err(CmdError::msg(
                "You have no specialty recorded on the abilities in that pool. \
                 Record one with +spec, or name one with `spec <name>`.",
            ));
        }
        specialty = true;
    }
    if let Some(name) = &args.spec_name {
        let ok = doc
            .as_ref()
            .map(|d| specialty_matches(d, &resolved.trait_fields, name))
            .unwrap_or(false);
        if !ok {
            return Err(CmdError::msg(format!(
                "No specialty {name:?} is recorded on the abilities in that pool."
            )));
        }
        specialty = true;
    }

    // Willpower spend (deduct a temporary point). The temporary pool is
    // bounded by permanent Willpower (crib: "0..permanent"); clamp the read so
    // a staff-set over-value self-heals rather than letting a character spend
    // more than their permanent rating.
    let mut willpower = false;
    if use_wp {
        let mut d = doc
            .clone()
            .ok_or_else(|| CmdError::msg("You need a sheet to spend Willpower."))?;
        let permanent = d.number("willpower").unwrap_or(1).max(0);
        let temp = d.number("willpower_temp").unwrap_or(0).clamp(0, permanent);
        if temp < 1 {
            return Err(CmdError::msg("You have no temporary Willpower to spend."));
        }
        d.set_number("willpower_temp", temp - 1);
        cmd.store_for_doc(&d)
            .put_by(ctx.w, who, &d, who)
            .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
        willpower = true;
    }

    // A Garou's current form shifts the Physical (and social) traits it names;
    // the delta is applied at roll time (never stored — a war form exceeds the
    // 5-dot cap). Returns 0 for any non-Werewolf sheet.
    let form = doc.as_ref().map_or(0, |d| {
        crate::splats::werewolf::current_form_pool_delta(d, &resolved.trait_fields)
    });
    let base = resolved.total + args.mod_delta + wound + form;
    let pool = base.max(1);
    let opts = Owod20Opts {
        difficulty: args.difficulty.unwrap_or(6),
        specialty,
        willpower,
        extra_successes: 0,
        again_tens_double: false,
    };
    Ok(Prepared {
        pool,
        wound,
        incap,
        opts,
    })
}

struct Prepared {
    pool: i32,
    wound: i32,
    incap: bool,
    opts: Owod20Opts,
}

/// A single (non-contested) roll.
fn single_roll(cmd: &WodCmd, ctx: &mut Ctx<'_>, args: &RollArgs) -> Result<(), CmdError> {
    let who = ctx.actor;
    let silent = ctx.switches.has("silent");

    // M13: an accepted Storyteller proposal makes the next roll ST-authoritative
    // — the pool + difficulty come from the ruling, not from what the player
    // typed. The +roll/silent is purely the trigger ("dice in your hands").
    // Consumed exactly once; an expired pending is dropped (a normal roll).
    let now = ctx.w.now();
    let pending = ctx
        .state_mut::<WodState>()
        .and_then(|st| st.take_pending(who, now));
    if let Some(p) = pending {
        return pending_roll(cmd, ctx, who, &p);
    }

    let priv_roll = ctx.switches.has("priv");
    let prep = prepare(
        cmd,
        ctx,
        who,
        args,
        ctx.switches.has("spec"),
        ctx.switches.has("wp"),
        ctx.switches.has("nowound"),
    )?;

    // Roll + log, holding the RNG borrow only for the roll itself.
    let now = ctx.w.now();
    let (outcome, ext_note) = {
        let st = ctx
            .state_mut::<WodState>()
            .ok_or_else(|| CmdError::msg("The WoD ruleset is not loaded."))?;
        let outcome = {
            let mut rctx = RollCtx::new(st.rng());
            Owod20Pool.roll(prep.pool, &prep.opts, &mut rctx)
        };
        let ext_note = args.ext_target.map(|target| {
            let ext = st.extended_for(who, target);
            let status = ext.add(outcome.clone());
            format!(
                " [extended {}/{}: {:?}]",
                ext.accumulated, ext.target, status
            )
        });
        st.push_log(
            who,
            RollLogEntry {
                who,
                expr: args.pool_expr.clone(),
                summary: outcome.summary(),
                difficulty: outcome.difficulty,
                ts: now,
                private: priv_roll,
                outcome: outcome.clone(),
            },
        );
        (outcome, ext_note)
    };

    let line = render_roll(ctx, who, args, &prep, &outcome, ext_note.as_deref());
    emit_roll(cmd, ctx, who, priv_roll, silent, &line, &outcome);

    if let Some(job) = args.job {
        attach_to_job(ctx, who, job, &line)?;
    }
    Ok(())
}

/// The M13 ST-authoritative roll: consume an accepted [`PendingAction`], roll
/// the RULING's pool at the RULING's difficulty (re-resolved through
/// [`resolve_sheet_pool`] so it can never exceed the sheet), log the outcome on
/// the seeded RNG, show the caster their own throw (the room stays silent — the
/// ST's interpretation delivers the in-fiction result), and defer the
/// interpretation pass carrying the ENGINE's logged outcome.
fn pending_roll(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    who: ObjId,
    pending: &crate::state::PendingAction,
) -> Result<(), CmdError> {
    // Re-resolve the ST pool against the live sheet (the sheet fence). Fall back
    // to the lenient resolver only if the sheet-only resolver cannot (e.g. a
    // sheetless actor), never widening past the sheet for a real character.
    let doc = cmd.read(ctx.w, who);
    let resolved = match &doc {
        Some(d) => crate::dice::resolve_sheet_pool(&cmd.schema_for_doc(d), d, &pending.pool)
            .or_else(|_| resolve_for(cmd, ctx, who, &pending.pool)),
        None => resolve_for(cmd, ctx, who, &pending.pool),
    }
    .map_err(CmdError::msg)?;

    // Casting is an action: the wound penalty and Garou form delta apply.
    let (wound, incap) = match &doc {
        Some(d) => {
            let track =
                crate::health::HealthTrack::from_marks(&d.track("health").unwrap_or_default());
            (
                i32::from(cmd.store_for_doc(d).wound_penalty(d, "health")),
                track.is_incapacitated(),
            )
        }
        None => (0, false),
    };
    let form = doc.as_ref().map_or(0, |d| {
        crate::splats::werewolf::current_form_pool_delta(d, &resolved.trait_fields)
    });
    let pool = (resolved.total + wound + form).max(1);
    let opts = Owod20Opts::new(pending.difficulty);

    let (log_label, verb) = match pending.kind {
        crate::st::ActionKind::Mage => ("cast", "works the effect"),
        crate::st::ActionKind::Generic => ("action", "attempts the action"),
    };
    let now = ctx.w.now();
    let outcome = {
        let st = ctx
            .state_mut::<WodState>()
            .ok_or_else(|| CmdError::msg("The WoD ruleset is not loaded."))?;
        let outcome = {
            let mut rctx = RollCtx::new(st.rng());
            Owod20Pool.roll(pool, &opts, &mut rctx)
        };
        st.push_log(
            who,
            RollLogEntry {
                who,
                expr: format!("{log_label}: {}", pending.pool),
                summary: outcome.summary(),
                difficulty: outcome.difficulty,
                ts: now,
                private: true,
                outcome: outcome.clone(),
            },
        );
        outcome
    };

    // The actor sees their own throw (they threw the dice); the room gets no
    // raw dice line — the result is narrated by the ST interpretation.
    let name = name_of(ctx, who);
    let mut line = format!(
        "{name} {verb} ({pool} dice, diff {}) and {}",
        pending.difficulty,
        outcome.summary()
    );
    if wound < 0 {
        line.push_str(&format!(" [wound {wound}]"));
    }
    if incap {
        line.push_str(" [incapacitated]");
    }
    ctx.out.sys_to_actor(who, "wod", line);

    // Defer the interpretation pass with the ENGINE's logged outcome + context.
    crate::st::defer_interpret(ctx, who, pending, &outcome)?;
    Ok(())
}

/// Resolve `expr` against `who`'s active sheet, roll it at `difficulty`
/// (optionally applying the wound penalty), log it, and deliver the result. The
/// reusable roll path splat commands (`+disc/roll`, `+frenzy`, `+cast`,
/// `+gift/roll`, `+cantrip`) build on — no specialty/willpower/contested
/// machinery.
///
/// `perc` selects the delivery policy (M14):
/// - `None` — a **mundane** roll: the classic line goes to the room (today's
///   behavior, unchanged for `+frenzy`, `+rage/frenzy`, plain resistance rolls).
/// - `Some(p)` — a **power** use: the roll is ACTOR-ONLY (no raw dice line to
///   the room, only a per-actor `wod.roll` panel keyframe), then it is routed
///   through [`project_perceptual_act`](crate::perception::project_perceptual_act)
///   so each onlooker perceives-or-not per `p`. `vulgar` additionally emits an
///   obvious-tier room-at-large line so a blatant act still reads to everyone.
///
/// # Errors
///
/// Returns a [`CmdError`] if the pool expression cannot be resolved.
#[allow(clippy::too_many_arguments)]
pub(crate) fn roll_expr(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    who: ObjId,
    expr: &str,
    difficulty: u8,
    apply_wound: bool,
    label: &str,
    perc: Option<&ActPerceptibility>,
    vulgar: bool,
) -> Result<Owod20Outcome, CmdError> {
    let resolved = resolve_for(cmd, ctx, who, expr).map_err(CmdError::msg)?;
    let doc = cmd.read(ctx.w, who);
    let (wound, incap) = match (&doc, apply_wound) {
        (Some(d), true) => {
            let track =
                crate::health::HealthTrack::from_marks(&d.track("health").unwrap_or_default());
            (
                i32::from(cmd.store_for_doc(d).wound_penalty(d, "health")),
                track.is_incapacitated(),
            )
        }
        _ => (0, false),
    };
    let form = doc.as_ref().map_or(0, |d| {
        crate::splats::werewolf::current_form_pool_delta(d, &resolved.trait_fields)
    });
    let pool = (resolved.total + wound + form).max(1);
    let opts = Owod20Opts::new(difficulty);
    let now = ctx.w.now();
    let outcome = {
        let st = ctx
            .state_mut::<WodState>()
            .ok_or_else(|| CmdError::msg("The WoD ruleset is not loaded."))?;
        let outcome = {
            let mut rctx = RollCtx::new(st.rng());
            Owod20Pool.roll(pool, &opts, &mut rctx)
        };
        st.push_log(
            who,
            RollLogEntry {
                who,
                expr: format!("{label}: {expr}"),
                summary: outcome.summary(),
                difficulty: outcome.difficulty,
                ts: now,
                private: false,
                outcome: outcome.clone(),
            },
        );
        outcome
    };
    let name = name_of(ctx, who);
    let mut line = format!(
        "{name} rolls {label} ({expr}, {pool} dice, diff {difficulty}) and {}",
        outcome.summary()
    );
    if wound < 0 {
        line.push_str(&format!(" [wound {wound}]"));
    }
    if incap {
        line.push_str(" [incapacitated]");
    }
    match perc {
        // A power use: no raw dice line to the room. The actor still gets a
        // `wod.roll` panel keyframe (they perceive their own act — that panel is
        // Actor-audience, never a room leak); the director then projects the act
        // per-observer and, if vulgar, emits the obvious-tier room line.
        Some(p) => {
            cmd.roll_feed.keyframe(
                ctx,
                serde_json::json!({
                    "who": who.to_string(),
                    "summary": line,
                    "outcome": outcome,
                }),
            );
            let npc_winners = crate::perception::project_perceptual_act(
                cmd.dispatch.as_ref(),
                ctx.w,
                ctx.out,
                who,
                p,
                &line,
                vulgar,
            );
            // M14 D6: the director resolved which onlookers perceived the power;
            // announce it so a witnessing NPC informant files intel. The event
            // carries the director's NPC winner set — the informant loop files
            // for exactly these, with NO perception re-gate.
            if let Some(room) = ctx.w.get(who).and_then(|o| o.location) {
                ctx.emit_world_event(&WorldEvent::PowerUsed {
                    who,
                    room,
                    witnesses: npc_winners,
                    detail: format!("used {label} ({expr})"),
                });
            }
        }
        // A mundane roll: the classic room-visible line (unchanged).
        None => emit_roll(cmd, ctx, who, false, false, &line, &outcome),
    }
    Ok(outcome)
}

/// A contested roll: the actor and a target each roll, then compare.
fn contested_roll(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    args: &RollArgs,
    target_spec: &str,
    their_expr: &str,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let target = match_object(ctx.w, actor, target_spec)?;
    let priv_roll = ctx.switches.has("priv");

    // Validate the opponent's pool expression *before* the actor side runs —
    // `prepare(mine)` deducts (and persists) a Willpower point as a side
    // effect, and a bad opponent pool must not burn the actor's point on a
    // roll that never resolves.
    resolve_for(cmd, ctx, target, their_expr).map_err(CmdError::msg)?;

    // Actor side (may spend willpower / specialty via switches).
    let mine = prepare(
        cmd,
        ctx,
        actor,
        args,
        ctx.switches.has("spec"),
        ctx.switches.has("wp"),
        ctx.switches.has("nowound"),
    )?;
    // Target side: same difficulty, no willpower/specialty switches applied.
    let their_args = RollArgs {
        pool_expr: their_expr.to_string(),
        difficulty: args.difficulty,
        ..RollArgs::default()
    };
    let theirs = prepare(cmd, ctx, target, &their_args, false, false, false)?;

    let now = ctx.w.now();
    let (out_a, out_b) = {
        let st = ctx
            .state_mut::<WodState>()
            .ok_or_else(|| CmdError::msg("The WoD ruleset is not loaded."))?;
        let a = {
            let mut rctx = RollCtx::new(st.rng());
            Owod20Pool.roll(mine.pool, &mine.opts, &mut rctx)
        };
        let b = {
            let mut rctx = RollCtx::new(st.rng());
            Owod20Pool.roll(theirs.pool, &theirs.opts, &mut rctx)
        };
        st.push_log(
            actor,
            RollLogEntry {
                who: actor,
                expr: format!("{} (contested)", args.pool_expr),
                summary: a.summary(),
                difficulty: a.difficulty,
                ts: now,
                private: priv_roll,
                outcome: a.clone(),
            },
        );
        (a, b)
    };

    let a_name = name_of(ctx, actor);
    let b_name = name_of(ctx, target);
    let result = contest(&out_a, &out_b);
    let winner = match result {
        mush_dice::owod20::Contest::Win => format!("{a_name} wins"),
        mush_dice::owod20::Contest::Lose => format!("{b_name} wins"),
        mush_dice::owod20::Contest::Deadlock => "deadlock (no clear winner)".to_string(),
    };
    let line = format!(
        "Contested roll: {a_name} [{}] {} vs {b_name} [{}] {} => {winner}.",
        args.pool_expr,
        out_a.summary(),
        their_expr,
        out_b.summary(),
    );
    emit_roll(
        cmd,
        ctx,
        actor,
        priv_roll,
        ctx.switches.has("silent"),
        &line,
        &out_a,
    );
    if let Some(job) = args.job {
        attach_to_job(ctx, actor, job, &line)?;
    }
    Ok(())
}

/// Render the classic one-liner for a single roll.
fn render_roll(
    ctx: &Ctx<'_>,
    who: ObjId,
    args: &RollArgs,
    prep: &Prepared,
    outcome: &Owod20Outcome,
    ext_note: Option<&str>,
) -> String {
    let name = name_of(ctx, who);
    let expr = if args.pool_expr.trim().is_empty() {
        "(pool)".to_string()
    } else {
        args.pool_expr.clone()
    };
    let mut s = format!(
        "{name} rolls {expr} ({} dice, diff {}) and {}",
        prep.pool,
        outcome.difficulty,
        outcome.summary()
    );
    if prep.wound < 0 {
        s.push_str(&format!(" [wound {}]", prep.wound));
    }
    if prep.incap {
        s.push_str(" [incapacitated]");
    }
    if let Some(n) = ext_note {
        s.push_str(n);
    }
    s
}

/// Deliver a roll line to the room (public) or the roller + in-room staff
/// (private), always also pushing a `wod.roll` panel keyframe to the roller.
///
/// `silent` (the `/silent` switch) is stronger than `priv_roll`: the line goes
/// to the roller alone — no room line and no staff copy — so no other observer
/// receives any event at all.
fn emit_roll(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    who: ObjId,
    priv_roll: bool,
    silent: bool,
    line: &str,
    outcome: &Owod20Outcome,
) {
    if silent {
        ctx.out.sys_to_actor(who, "wod", line);
    } else if priv_roll {
        ctx.out.sys_to_actor(who, "wod", line);
        // In-room staff also see private rolls.
        if let Some(loc) = ctx.w.get(who).and_then(|o| o.location) {
            let staff: Vec<ObjId> = ctx
                .w
                .get(loc)
                .map(|r| r.contents.clone())
                .unwrap_or_default()
                .into_iter()
                .filter(|&o| o != who && perm::has_role(ctx.w, o, Role::Royalty))
                .collect();
            for s in staff {
                ctx.out.sys_to_actor(s, "wod", format!("(private) {line}"));
            }
        }
    } else if let Some(loc) = ctx.w.get(who).and_then(|o| o.location) {
        ctx.out.to_room(
            loc,
            Vec::new(),
            OutputEvent::Narrative {
                kind: NarrativeKind::Emit,
                actor: Some(mush_core::obj_ref(ctx.w, who)),
                text: RichText::plain(line),
            },
        );
    } else {
        ctx.out.sys_to_actor(who, "wod", line);
    }
    // Panel keyframe with full per-die provenance.
    cmd.roll_feed.keyframe(
        ctx,
        serde_json::json!({
            "who": who.to_string(),
            "summary": line,
            "outcome": outcome,
        }),
    );
}

/// `+roll/last [<target>]` — replay recent rolls (own, or a target's for staff).
fn roll_last(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim();
    let target = if raw.is_empty() {
        ctx.actor
    } else {
        let t = match_object(ctx.w, ctx.actor, raw)?;
        if t != ctx.actor && !perm::has_role(ctx.w, ctx.actor, Role::Royalty) {
            return Err(CmdError::msg("Only staff can read another's roll log."));
        }
        t
    };
    let lines: Vec<String> = {
        let st = ctx
            .state_mut::<WodState>()
            .ok_or_else(|| CmdError::msg("The WoD ruleset is not loaded."))?;
        st.recent_logs(target, 10)
            .iter()
            .map(|e| {
                let vis = if e.private { " (private)" } else { "" };
                format!("  [{}]{vis} {}", e.expr, e.summary)
            })
            .collect()
    };
    let name = name_of(ctx, target);
    if lines.is_empty() {
        ctx.out
            .info(ctx.actor, format!("{name} has no recent rolls."));
    } else {
        ctx.out
            .sys_to_actor(ctx.actor, "wod", format!("Recent rolls for {name}:"));
        for l in lines {
            ctx.out.sys_to_actor(ctx.actor, "wod", l);
        }
    }
    Ok(())
}

/// Pin a roll's summary onto job `id` as a comment.
fn attach_to_job(ctx: &mut Ctx<'_>, who: ObjId, id: u64, line: &str) -> Result<(), CmdError> {
    if mush_jobs::get_job(ctx.w, id).is_none() {
        return Err(CmdError::msg(format!("No such job: {id}.")));
    }
    mush_jobs::add_comment(ctx, id, who, &format!("[roll] {line}")).map_err(CmdError::msg)?;
    ctx.out
        .info(ctx.actor, format!("Roll attached to job {id}."));
    Ok(())
}

/// Whether any of `fields` (ability ids) has *any* recorded specialty — the
/// gate for the bare `/spec` switch (crib: 10-again needs an applicable
/// specialty, not a free toggle). Reused by the NPC `wod_roll` ability so an
/// agent cannot claim a specialty it has not recorded.
pub(crate) fn has_any_specialty(doc: &mush_ruleset::SheetDoc, fields: &[String]) -> bool {
    let Some(specs) = doc.map("specialties") else {
        return false;
    };
    fields.iter().any(|f| {
        specs
            .get(f)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|arr| arr.iter().any(serde_json::Value::is_string))
    })
}

/// Whether any of `fields` (ability ids) records the specialty `name`.
fn specialty_matches(doc: &mush_ruleset::SheetDoc, fields: &[String], name: &str) -> bool {
    let Some(specs) = doc.map("specialties") else {
        return false;
    };
    fields.iter().any(|f| {
        specs
            .get(f)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|arr| {
                arr.iter()
                    .filter_map(serde_json::Value::as_str)
                    .any(|s| s.eq_ignore_ascii_case(name))
            })
    })
}

fn name_of(ctx: &Ctx<'_>, who: ObjId) -> String {
    ctx.w.get(who).map_or_else(
        || who.to_string(),
        |o| mush_core::output::primary_name(&o.name).to_string(),
    )
}

fn fmt_errs(errs: &[mush_ruleset::SheetError]) -> String {
    errs.iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Register `+roll` under the calling module.
pub(crate) fn register(
    reg: &mut mush_core::Registrar<'_>,
    cmd: Arc<WodCmd>,
) -> Result<(), mush_core::ModError> {
    use mush_core::{ArgShape, CommandSpec};
    reg.command(
        CommandSpec {
            name: "+roll".into(),
            switches: vec![
                "spec".into(),
                "wp".into(),
                "priv".into(),
                "silent".into(),
                "nowound".into(),
                "last".into(),
            ],
            shape: ArgShape::Raw,
            help: "+roll <pool> [vs <diff>]  --  Roll a Storyteller pool. <pool> is \
                   a trait expression resolved against your sheet (strength+brawl+2) \
                   or a plain number. Difficulty defaults to 6.\n\
                   Switches: /spec (10s count double — needs a specialty recorded \
                   on one of the pool's abilities), /wp (spend Willpower for +1 \
                   success), /priv (result to you and staff only), /silent (result \
                   to you alone — no room line, no staff copy), /nowound (ignore \
                   the wound penalty).\n\
                   Inline options: vs <n> / diff <n>, mod <n>, spec <name> (a \
                   specialty on your sheet), ext <n> (extended toward n successes), \
                   job <n> (pin to a job), contested <target>=<pool>.\n\
                   +roll/last [<target>] shows recent rolls (staff for others)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| roll_cmd(&cmd, ctx))
        },
    )
}
