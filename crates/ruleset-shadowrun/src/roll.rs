//! `+roll` — the SR5 d6 hit-pool command — plus the roll-log ring
//! (`+roll/last`) and the `sr.roll` panel feed.
//!
//! `+roll <attr>+<skill>[+/-mod]` resolves a trait pool (or a raw integer) from
//! the runner's sheet, auto-applies the wound modifier and the sensible Limit,
//! rolls through [`Sr5Pool`](mush_dice::sr5::Sr5Pool) with the server RNG, and
//! renders the classic hits / glitch result to the room (unless `/priv`), to a
//! per-character transient roll-log ring, and to the `sr.roll` panel with full
//! per-die records.
//!
//! Switches: `/edge` (spend Edge to **push the limit** — add Edge dice, ignore
//! the Limit, and explode 6s), `/edge+` (just add Edge dice), `/priv`, `/nowound`
//! (skip the wound modifier). Inline options: `threshold <n>` / `vs <n>`,
//! `limit <physical|mental|social|n|none>`, `mod <n>`, `spec <name>`,
//! `contested <target>=<pool>`, `extended <n>`.

use std::sync::Arc;

use mush_core::{CmdError, Ctx, ObjId, Role, match_object, perm};
use mush_dice::RollCtx;
use mush_dice::pool::PoolMechanic;
use mush_dice::sr5::{Sr5Opts, Sr5Outcome, Sr5Pool, contest};
use mush_proto::{NarrativeKind, OutputEvent, RichText};

use crate::Sr5Cmd;
use crate::dice::{LimitKind, PoolTerm, ResolvedPool, resolve_pool};
use crate::schema::{EDGE, EDGE_CURRENT};
use crate::state::{RollLogEntry, Sr5State};

/// A parsed Limit argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LimitArg {
    /// `limit none` — waive the Limit.
    None,
    /// `limit <n>` — an explicit numeric Limit.
    Num(u8),
    /// `limit physical|mental|social` — a named derived Limit.
    Kind(LimitKind),
}

/// Parsed `+roll` options (dispatch switches are read from `ctx`).
#[derive(Clone, Debug, Default)]
struct RollArgs {
    pool_expr: String,
    threshold: Option<u8>,
    mod_delta: i32,
    limit: Option<LimitArg>,
    spec_name: Option<String>,
    ext_target: Option<u32>,
    contested: Option<(String, String)>,
}

fn is_keyword(t: &str) -> bool {
    matches!(
        t.to_ascii_lowercase().as_str(),
        "threshold" | "thresh" | "vs" | "mod" | "limit" | "spec" | "contested" | "extended" | "ext"
    )
}

/// Parse the `+roll` argument: pool expression, then inline keyword options.
fn parse_roll_args(raw: &str) -> Result<RollArgs, CmdError> {
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    let mut args = RollArgs::default();
    let mut pool = Vec::new();
    let mut i = 0;
    while i < tokens.len() && !is_keyword(tokens[i]) {
        pool.push(tokens[i]);
        i += 1;
    }
    args.pool_expr = pool.join(" ");
    while i < tokens.len() {
        let kw = tokens[i].to_ascii_lowercase();
        i += 1;
        match kw.as_str() {
            "threshold" | "thresh" | "vs" => {
                let v = tokens
                    .get(i)
                    .ok_or_else(|| CmdError::msg("threshold needs a number."))?;
                args.threshold = Some(
                    v.parse::<u8>()
                        .map_err(|_| CmdError::msg(format!("Bad threshold: {v:?}")))?,
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
            "limit" => {
                let v = tokens
                    .get(i)
                    .ok_or_else(|| CmdError::msg("limit needs physical/mental/social/<n>/none."))?;
                args.limit = Some(parse_limit_arg(v)?);
                i += 1;
            }
            "spec" => {
                let v = tokens
                    .get(i)
                    .ok_or_else(|| CmdError::msg("spec needs a name."))?;
                args.spec_name = Some((*v).to_string());
                i += 1;
            }
            "extended" | "ext" => {
                let v = tokens
                    .get(i)
                    .ok_or_else(|| CmdError::msg("extended needs a hit threshold."))?;
                args.ext_target = Some(
                    v.parse::<u32>()
                        .map_err(|_| CmdError::msg(format!("Bad extended threshold: {v:?}")))?,
                );
                i += 1;
            }
            "contested" => {
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

fn parse_limit_arg(v: &str) -> Result<LimitArg, CmdError> {
    if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("no") {
        return Ok(LimitArg::None);
    }
    if let Ok(n) = v.parse::<u8>() {
        return Ok(LimitArg::Num(n));
    }
    LimitKind::parse(v).map(LimitArg::Kind).ok_or_else(|| {
        CmdError::msg(format!(
            "Bad limit {v:?} (physical/mental/social/<n>/none)."
        ))
    })
}

/// Resolve a pool against `who`'s sheet (or a raw integer if they have none).
fn resolve_for(
    cmd: &Sr5Cmd,
    ctx: &Ctx<'_>,
    who: ObjId,
    expr: &str,
) -> Result<ResolvedPool, String> {
    match cmd.read(ctx.w, who) {
        Some(doc) => resolve_pool(&cmd.schema, &doc, expr),
        None => {
            let n: i32 = expr
                .trim()
                .parse()
                .map_err(|_| "That character has no sheet; roll a plain number.".to_string())?;
            Ok(ResolvedPool {
                terms: vec![PoolTerm {
                    label: "pool".to_string(),
                    value: n,
                }],
                total: n,
                skill_fields: Vec::new(),
                limit: None,
            })
        }
    }
}

/// The assembled roll parameters after resolving the sheet, limit, wound, and
/// Edge.
struct Prepared {
    pool: i32,
    opts: Sr5Opts,
    wound: i32,
    limit_note: Option<String>,
    edge_note: Option<String>,
    spec_applied: bool,
}

/// Assemble roll options for `who` rolling `resolved` under `args` + switches,
/// performing the Edge deduction (a sheet write) as a side effect.
#[allow(clippy::too_many_arguments)]
fn prepare(
    cmd: &Sr5Cmd,
    ctx: &mut Ctx<'_>,
    who: ObjId,
    args: &RollArgs,
    resolved: &ResolvedPool,
    edge_push: bool,
    edge_dice_only: bool,
    nowound: bool,
) -> Result<Prepared, CmdError> {
    let doc = cmd.read(ctx.w, who);

    // Wound modifier (<= 0) from the Condition Monitors.
    let wound = match (&doc, nowound) {
        (Some(d), false) => crate::derive::wound_modifier(d) as i32,
        _ => 0,
    };

    // Specialization (+2 dice) — only if actually recorded on a pool skill.
    let mut spec_applied = false;
    if let Some(name) = &args.spec_name {
        let ok = doc
            .as_ref()
            .is_some_and(|d| crate::dice::specialization_matches(d, &resolved.skill_fields, name));
        if !ok {
            return Err(CmdError::msg(format!(
                "No specialization {name:?} is recorded on the skills in that pool."
            )));
        }
        spec_applied = true;
    }

    // Edge spend (push the limit, or just add dice). Deducts one temp Edge.
    let mut edge_note = None;
    let mut edge_dice = 0u8;
    if edge_push || edge_dice_only {
        let mut d = doc
            .clone()
            .ok_or_else(|| CmdError::msg("You need a sheet to spend Edge."))?;
        let rating = d.number(EDGE).unwrap_or(0).max(0);
        let temp = d.number(EDGE_CURRENT).unwrap_or(0).clamp(0, rating);
        if temp < 1 {
            return Err(CmdError::msg(
                "You have no Edge left to spend this session.",
            ));
        }
        d.set_number(EDGE_CURRENT, temp - 1);
        cmd.store()
            .put_by(ctx.w, who, &d, who)
            .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
        edge_dice = rating.clamp(0, i64::from(u8::MAX)) as u8;
        edge_note = Some(if edge_push {
            format!("pushed the limit (+{edge_dice} Edge dice, Rule of Six)")
        } else {
            format!("spent Edge (+{edge_dice} dice)")
        });
    }

    // Limit: explicit arg, else the inferred one; waived when pushing.
    let (limit, limit_note) = if edge_push {
        (None, Some("limit ignored".to_string()))
    } else {
        resolve_limit(args.limit, resolved, doc.as_ref())
    };

    let spec_bonus = if spec_applied { 2 } else { 0 };
    let opts = Sr5Opts {
        threshold: args.threshold.unwrap_or(1),
        limit,
        edge_push,
        edge_dice,
        modifier: args.mod_delta + wound + spec_bonus,
    };
    Ok(Prepared {
        pool: resolved.total,
        opts,
        wound,
        limit_note,
        edge_note,
        spec_applied,
    })
}

/// Resolve the effective Limit and a note for the render.
fn resolve_limit(
    arg: Option<LimitArg>,
    resolved: &ResolvedPool,
    doc: Option<&mush_ruleset::SheetDoc>,
) -> (Option<u8>, Option<String>) {
    let clamp = |n: i64| n.clamp(0, i64::from(u8::MAX)) as u8;
    match arg {
        Some(LimitArg::None) => (None, None),
        Some(LimitArg::Num(n)) => (Some(n), Some(format!("limit {n}"))),
        Some(LimitArg::Kind(k)) => {
            let v = doc.map_or(0, |d| clamp(k.value(d)));
            (Some(v), Some(format!("{} limit {v}", k.label())))
        }
        None => match (resolved.limit, doc) {
            (Some(k), Some(d)) => {
                let v = clamp(k.value(d));
                (Some(v), Some(format!("{} limit {v}", k.label())))
            }
            _ => (None, None),
        },
    }
}

/// `+roll` entry point.
pub(crate) fn roll_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("last") {
        return roll_last(ctx);
    }
    let args = parse_roll_args(ctx.args.raw())?;
    if let Some((target_spec, their_expr)) = args.contested.clone() {
        return contested_roll(cmd, ctx, &args, &target_spec, &their_expr);
    }
    single_roll(cmd, ctx, &args)
}

/// A single (non-contested) roll.
fn single_roll(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>, args: &RollArgs) -> Result<(), CmdError> {
    let who = ctx.actor;
    let priv_roll = ctx.switches.has("priv");
    let resolved = resolve_for(cmd, ctx, who, &args.pool_expr).map_err(CmdError::msg)?;
    let prep = prepare(
        cmd,
        ctx,
        who,
        args,
        &resolved,
        ctx.switches.has("edge"),
        ctx.switches.has("edge+"),
        ctx.switches.has("nowound"),
    )?;

    let now = ctx.w.now();
    let (outcome, ext_note) = {
        let st = ctx
            .state_mut::<Sr5State>()
            .ok_or_else(|| CmdError::msg("The Shadowrun ruleset is not loaded."))?;
        let (outcome, ext_note) = match args.ext_target {
            // Extended test: an SR5 extended test loses one die from the pool
            // each interval. The accumulator (seeded with the base pool) tracks
            // the shrinking size in `next_pool`; roll THIS interval at that size
            // (modifiers still apply on top), then accumulate.
            Some(target) => {
                let this_pool = st.extended_for(who, target, prep.pool).next_pool;
                let outcome = {
                    let mut rctx = RollCtx::new(st.rng());
                    Sr5Pool.roll(this_pool, &prep.opts, &mut rctx)
                };
                let ext = st.extended_for(who, target, prep.pool);
                let status = ext.add(outcome.clone());
                let note = format!(
                    " [extended {}/{} hits, {} dice this interval: {status:?}]",
                    ext.accumulated,
                    ext.threshold,
                    (this_pool + prep.opts.modifier).max(0),
                );
                (outcome, Some(note))
            }
            None => {
                let outcome = {
                    let mut rctx = RollCtx::new(st.rng());
                    Sr5Pool.roll(prep.pool, &prep.opts, &mut rctx)
                };
                (outcome, None)
            }
        };
        st.push_log(
            who,
            RollLogEntry {
                who,
                expr: args.pool_expr.clone(),
                summary: outcome.summary(),
                threshold: prep.opts.threshold,
                ts: now,
                private: priv_roll,
                outcome: outcome.clone(),
            },
        );
        (outcome, ext_note)
    };

    let line = render_roll(ctx, who, &resolved, &prep, &outcome, ext_note.as_deref());
    emit_roll(cmd, ctx, who, priv_roll, &line, &outcome);
    Ok(())
}

/// Resolve `expr` against `who`'s sheet and roll it at the given `threshold` and
/// `limit`, applying the wound modifier, logging, and emitting the classic line.
/// The reusable roll path `+cast` / `+init` build on.
///
/// # Errors
///
/// Returns a [`CmdError`] if the pool expression cannot be resolved.
#[allow(clippy::too_many_arguments)]
pub(crate) fn roll_labeled(
    cmd: &Sr5Cmd,
    ctx: &mut Ctx<'_>,
    who: ObjId,
    expr: &str,
    label: &str,
    threshold: u8,
    limit: Option<u8>,
    extra_mod: i32,
    apply_wound: bool,
) -> Result<Sr5Outcome, CmdError> {
    let resolved = resolve_for(cmd, ctx, who, expr).map_err(CmdError::msg)?;
    let doc = cmd.read(ctx.w, who);
    let wound = match (&doc, apply_wound) {
        (Some(d), true) => crate::derive::wound_modifier(d) as i32,
        _ => 0,
    };
    let opts = Sr5Opts {
        threshold,
        limit,
        edge_push: false,
        edge_dice: 0,
        modifier: extra_mod + wound,
    };
    let now = ctx.w.now();
    let outcome = {
        let st = ctx
            .state_mut::<Sr5State>()
            .ok_or_else(|| CmdError::msg("The Shadowrun ruleset is not loaded."))?;
        let outcome = {
            let mut rctx = RollCtx::new(st.rng());
            Sr5Pool.roll(resolved.total, &opts, &mut rctx)
        };
        st.push_log(
            who,
            RollLogEntry {
                who,
                expr: format!("{label}: {expr}"),
                summary: outcome.summary(),
                threshold,
                ts: now,
                private: false,
                outcome: outcome.clone(),
            },
        );
        outcome
    };
    let name = name_of(ctx, who);
    let dice = (resolved.total + opts.modifier).max(0);
    let mut line = format!(
        "{name} rolls {label} ({expr}, {dice} dice) and {}",
        outcome.summary()
    );
    if wound < 0 {
        line.push_str(&format!(" [wound {wound}]"));
    }
    emit_roll(cmd, ctx, who, false, &line, &outcome);
    Ok(outcome)
}

/// A contested roll: the actor and a target each roll, then compare net hits.
fn contested_roll(
    cmd: &Sr5Cmd,
    ctx: &mut Ctx<'_>,
    args: &RollArgs,
    target_spec: &str,
    their_expr: &str,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let target = match_object(ctx.w, actor, target_spec)?;
    let priv_roll = ctx.switches.has("priv");

    // Validate both pools before the actor's Edge is spent.
    let mine_resolved = resolve_for(cmd, ctx, actor, &args.pool_expr).map_err(CmdError::msg)?;
    let their_resolved = resolve_for(cmd, ctx, target, their_expr).map_err(CmdError::msg)?;

    // Prepare the *opponent's* pool first (it never spends Edge). `prepare` for
    // the actor performs the Edge deduction as a side effect, so it must run
    // last — otherwise a failure resolving the opponent would leave the actor's
    // Edge already spent with no roll made.
    let their_args = RollArgs {
        pool_expr: their_expr.to_string(),
        ..RollArgs::default()
    };
    let theirs = prepare(
        cmd,
        ctx,
        target,
        &their_args,
        &their_resolved,
        false,
        false,
        false,
    )?;
    let mine = prepare(
        cmd,
        ctx,
        actor,
        args,
        &mine_resolved,
        ctx.switches.has("edge"),
        ctx.switches.has("edge+"),
        ctx.switches.has("nowound"),
    )?;

    let now = ctx.w.now();
    let (out_a, out_b) = {
        let st = ctx
            .state_mut::<Sr5State>()
            .ok_or_else(|| CmdError::msg("The Shadowrun ruleset is not loaded."))?;
        let a = {
            let mut rctx = RollCtx::new(st.rng());
            Sr5Pool.roll(mine.pool, &mine.opts, &mut rctx)
        };
        let b = {
            let mut rctx = RollCtx::new(st.rng());
            Sr5Pool.roll(theirs.pool, &theirs.opts, &mut rctx)
        };
        st.push_log(
            actor,
            RollLogEntry {
                who: actor,
                expr: format!("{} (contested)", args.pool_expr),
                summary: a.summary(),
                threshold: mine.opts.threshold,
                ts: now,
                private: priv_roll,
                outcome: a.clone(),
            },
        );
        (a, b)
    };

    let a_name = name_of(ctx, actor);
    let b_name = name_of(ctx, target);
    let verdict = match contest(&out_a, &out_b) {
        mush_dice::sr5::Sr5Contest::ActorWins(n) => format!("{a_name} wins by {n} net hit(s)"),
        mush_dice::sr5::Sr5Contest::OpponentWins(n) => format!("{b_name} wins by {n} net hit(s)"),
        mush_dice::sr5::Sr5Contest::Tie => "a tie (defender prevails)".to_string(),
    };
    let line = format!(
        "Opposed test: {a_name} [{}] {} vs {b_name} [{}] {} => {verdict}.",
        args.pool_expr,
        out_a.summary(),
        their_expr,
        out_b.summary(),
    );
    emit_roll(cmd, ctx, actor, priv_roll, &line, &out_a);
    Ok(())
}

/// Render the classic one-liner for a single roll.
fn render_roll(
    ctx: &Ctx<'_>,
    who: ObjId,
    resolved: &ResolvedPool,
    prep: &Prepared,
    outcome: &Sr5Outcome,
    ext_note: Option<&str>,
) -> String {
    let name = name_of(ctx, who);
    let expr = if resolved.terms.is_empty() {
        "(pool)".to_string()
    } else {
        resolved
            .terms
            .iter()
            .map(|t| t.label.clone())
            .collect::<Vec<_>>()
            .join("+")
    };
    let dice = (prep.pool + prep.opts.modifier).max(0) + i32::from(prep.opts.edge_dice);
    let mut s = format!(
        "{name} rolls {expr} ({dice} dice) and {}",
        outcome.summary()
    );
    if prep.spec_applied {
        s.push_str(" [specialization +2]");
    }
    if prep.wound < 0 {
        s.push_str(&format!(" [wound {}]", prep.wound));
    }
    if let Some(n) = &prep.edge_note {
        s.push_str(&format!(" [{n}]"));
    } else if let Some(n) = &prep.limit_note {
        s.push_str(&format!(" [{n}]"));
    }
    if let Some(n) = ext_note {
        s.push_str(n);
    }
    s
}

/// Deliver a roll line to the room (public) or the roller + in-room staff
/// (private), always pushing an `sr.roll` panel keyframe to the roller.
fn emit_roll(
    cmd: &Sr5Cmd,
    ctx: &mut Ctx<'_>,
    who: ObjId,
    priv_roll: bool,
    line: &str,
    outcome: &Sr5Outcome,
) {
    if priv_roll {
        ctx.out.sys_to_actor(who, "sr", line);
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
                ctx.out.sys_to_actor(s, "sr", format!("(private) {line}"));
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
        ctx.out.sys_to_actor(who, "sr", line);
    }
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
            .state_mut::<Sr5State>()
            .ok_or_else(|| CmdError::msg("The Shadowrun ruleset is not loaded."))?;
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
            .sys_to_actor(ctx.actor, "sr", format!("Recent rolls for {name}:"));
        for l in lines {
            ctx.out.sys_to_actor(ctx.actor, "sr", l);
        }
    }
    Ok(())
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

/// Register `+roll`.
pub(crate) fn register(
    reg: &mut mush_core::Registrar<'_>,
    cmd: Arc<Sr5Cmd>,
) -> Result<(), mush_core::ModError> {
    use mush_core::{ArgShape, CommandSpec};
    reg.command(
        CommandSpec {
            name: "+roll".into(),
            switches: vec![
                "edge".into(),
                "edge+".into(),
                "priv".into(),
                "nowound".into(),
                "last".into(),
            ],
            shape: ArgShape::Raw,
            help: "+roll <attr>+<skill>[+/-mod]  --  Roll an SR5 d6 hit pool (5s and \
                   6s are hits). <pool> resolves against your sheet (body+pistols) or \
                   a plain number. More than half 1s glitch; zero hits then is a critical \
                   glitch. Your wound modifier and the sensible Limit apply \
                   automatically.\n\
                   Switches: /edge (spend Edge to push the limit — add Edge dice, ignore \
                   the Limit, and explode 6s), /edge+ (just add Edge dice), /priv \
                   (result to you and staff), /nowound (ignore the wound modifier).\n\
                   Inline: threshold <n> / vs <n>, limit <physical|mental|social|n|none>, \
                   mod <n>, spec <name> (a specialization you have), contested \
                   <target>=<pool>, extended <n> (accumulate toward n hits).\n\
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
