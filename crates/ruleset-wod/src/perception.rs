//! # M11 perception vertical — Obfuscate (hide) vs Auspex (notice)
//!
//! The WoD half of the engine's per-observer output projection. The core layer
//! ([`mush_core::can_perceive`]) asks a registered [`EventHook::perceives`]
//! whether one observer may perceive one subject; this module answers that
//! question for **Kindred concealment**:
//!
//! - a vampire (or ghoul) with **Obfuscate** runs `+obfuscate on` to fade from
//!   sight — [`WodState::conceal`] marks them, and every observer present rolls
//!   **once** to notice;
//! - an observer's chance to pierce the veil is **Perception + Awareness +
//!   Auspex** at a difficulty set by the concealer's Obfuscate; only a sense
//!   like **Auspex** lets a watcher try at all (no Auspex ⇒ they simply do not
//!   see, no dice spent);
//! - the verdict is cached in [`WodState`] ([`WodState::set_perception`]) and
//!   the read-only [`PerceptionHook`] is a pure lookup — the hot fan-out path
//!   never rolls;
//! - [`PerceptionObserver`] keeps the cache correct as people move, connect and
//!   disconnect: a new watcher entering a room rolls against anyone concealed
//!   there, a concealed subject moving re-rolls the new room, and leaving /
//!   disconnecting drops the stale verdicts.
//!
//! ## Fail-safe
//!
//! A concealed subject with **no cached verdict** for an observer is treated as
//! *concealed* — the gate never leaks on a race (an observer the maintenance
//! path has not rolled yet). An observer always perceives themselves.
//!
//! ## Determinism / additivity
//!
//! Rolls happen **only while concealment is active**: with nobody cloaked, the
//! observer no-ops before touching the RNG and the hook returns
//! [`Perceive::Abstain`] for everyone, so a game that never uses Obfuscate
//! advances no dice and behaves exactly as it did before M11.

use std::sync::Arc;

use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, EventHook, HookPoint, ModError, ObjId, ObjKind, Outbox,
    Perceive, Registrar, World, WorldEvent, WorldEventKind, WorldObserver, obj_ref,
};
use mush_dice::RollCtx;
use mush_dice::owod20::{Owod20Opts, Owod20Pool};
use mush_dice::pool::PoolMechanic;
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use mush_ruleset::{SheetDoc, SheetStore};
use serde::{Deserialize, Serialize};

use crate::WodCmd;
use crate::dice::ResolvedPool;
use crate::splats::Dispatch;
use crate::splats::vampire::discipline_rating;
use crate::state::{RollLogEntry, WodState};

/// The read-only per-observer gate. Registered under [`HookPoint::Perceive`];
/// consulted by [`mush_core::can_perceive`] from the look/examine filters and
/// the live room fan-out.
///
/// Pure lookup by contract (the shared `&World` is read-only): it never rolls
/// or writes the cache — that is [`PerceptionObserver`]'s and `+obfuscate`'s
/// job. It conceals a subject from an observer exactly when the subject is
/// concealed and the cached verdict is *not* "pierced" (absent ⇒ concealed,
/// the fail-safe).
pub struct PerceptionHook;

impl EventHook for PerceptionHook {
    fn perceives(&self, w: &World, observer: ObjId, subject: ObjId) -> Perceive {
        // You always perceive yourself.
        if observer == subject {
            return Perceive::Abstain;
        }
        let Some(st) = w.module_state::<WodState>() else {
            return Perceive::Abstain;
        };
        if !st.is_concealed(subject) {
            return Perceive::Abstain;
        }
        match st.perception_verdict(observer, subject) {
            Some(true) => Perceive::Abstain, // pierced the veil
            Some(false) | None => Perceive::Conceal, // failed, or not yet rolled (fail-safe)
        }
    }

    /// The ruleset-agnostic notability signal (see
    /// [`mush_core::is_concealed`]): `subject` is concealed exactly when it is
    /// under active Obfuscate. Observer-independent — a pure read of the
    /// concealment set, no roll, no cache write.
    fn is_concealed(&self, w: &World, subject: ObjId) -> bool {
        w.module_state::<WodState>()
            .is_some_and(|st| st.is_concealed(subject))
    }
}

/// Keeps the pierced-verdict cache correct across presence changes. Registered
/// for [`WorldEventKind::Moved`] / `Connected` / `Disconnected`.
pub struct PerceptionObserver {
    cmd: Arc<WodCmd>,
}

impl WorldObserver for PerceptionObserver {
    fn on_event(&self, ctx: &mut Ctx<'_>, ev: &WorldEvent) {
        // Fast path: nothing concealed ⇒ nothing to maintain, no RNG touched.
        if ctx
            .state::<WodState>()
            .is_none_or(|st| !st.has_concealment())
        {
            return;
        }
        match *ev {
            WorldEvent::Moved { who, .. } => {
                // `who` as a concealed subject that changed rooms: its old
                // verdicts are stale, re-roll the room it is now in.
                if let Some(rating) = ctx.state::<WodState>().and_then(|st| st.concealed_rating(who))
                {
                    if let Some(st) = ctx.state_mut::<WodState>() {
                        st.clear_perceptions_for_subject(who);
                    }
                    refresh_subject(&self.cmd, ctx, who, rating);
                }
                // `who` as an observer that changed rooms: drop its stale
                // verdicts and re-roll against anyone concealed where it landed.
                if let Some(st) = ctx.state_mut::<WodState>() {
                    st.clear_perceptions_by_observer(who);
                }
                refresh_observer(&self.cmd, ctx, who);
            }
            WorldEvent::Connected { who } => {
                // A returning watcher: roll it against anyone concealed here.
                refresh_observer(&self.cmd, ctx, who);
            }
            WorldEvent::Disconnected { who } => {
                // A cloaked actor stays concealed across a disconnect — they
                // never revealed. Crucially, their own "has disconnected." line
                // is still queued in this same step and delivered *after* the
                // observers run, so the concealment mark and the verdicts ABOUT
                // them must survive here or that line would leak to observers
                // who could not perceive them (I2). We only drop the verdicts
                // this actor held as an OBSERVER — it re-rolls those on
                // reconnect (Connected → refresh_observer).
                if let Some(st) = ctx.state_mut::<WodState>() {
                    st.clear_perceptions_by_observer(who);
                }
            }
            _ => {}
        }
    }
}

/// Whether `obj` is something that can watch (a player or an NPC).
fn is_observer(w: &World, obj: ObjId) -> bool {
    matches!(
        w.get(obj).map(|o| o.kind),
        Some(ObjKind::Player | ObjKind::Npc)
    )
}

/// Read `observer`'s sensing dots: `(Perception, Awareness, Auspex)`.
fn observer_senses(cmd: &WodCmd, w: &World, observer: ObjId) -> (i32, i32, i32) {
    let Some(doc) = cmd.read(w, observer) else {
        return (0, 0, 0);
    };
    let perception = doc.number("perception").unwrap_or(0) as i32;
    let awareness = doc.number("awareness").unwrap_or(0) as i32;
    let auspex = discipline_rating(&doc, "Auspex");
    (perception, awareness, auspex)
}

/// The headless, non-emitting perception roll core: does `observer` (with the
/// given sensing dots) pierce a veil of Obfuscate `rating`?
///
/// Rolls on the shared seeded RNG and records a **private** roll-log entry (so
/// staff can audit it), but touches no sheet and emits no room output — the
/// only side effects are the RNG draw and the log. An observer with **no
/// Auspex cannot pierce at all** and spends no dice (so non-Auspex presence in
/// a room of cloaked Kindred advances no RNG). Pierced ⇔ at least one success.
fn perception_roll(
    st: &mut WodState,
    observer: ObjId,
    perception: i32,
    awareness: i32,
    auspex: i32,
    obfuscate: i32,
    now: i64,
) -> bool {
    if auspex < 1 {
        // No preternatural sense: the veil holds, no roll made.
        return false;
    }
    let pool = perception + awareness + auspex;
    // Difficulty scales with the concealer's Obfuscate, clamped to the legal
    // oWoD 2..=10 band. Obfuscate 1 ⇒ diff 4, Obfuscate 5 ⇒ diff 8.
    let difficulty = (3 + obfuscate).clamp(2, 10) as u8;
    let expr = format!("perception+awareness+auspex vs Obfuscate {obfuscate}");
    sense_roll(st, observer, pool, difficulty, &expr, now, true).successes() > 0
}

/// The headless sensing-roll core: roll `pool` dice at `difficulty` on the
/// shared seeded RNG and return the full [`Owod20Outcome`]. No sheet touch, no
/// room output — the only side effects are the RNG draw and (when `log`) a
/// **private** roll-log entry. Shared by the M11 concealment pierce
/// ([`perception_roll`], which logs so staff can audit a pierce) and the M14
/// silent-witness resolution ([`resolve_action_witnesses`], which passes
/// `log = false`: a per-observer witness roll must leave **no** retrievable
/// artifact, or a non-perceiver could read the witness difficulty (and that an
/// act happened) out of their own `+roll/last` — the V1 no-leak invariant). The
/// RNG draw happens either way, so seeded determinism is identical regardless of
/// `log`.
pub(crate) fn sense_roll(
    st: &mut WodState,
    observer: ObjId,
    pool: i32,
    difficulty: u8,
    expr: &str,
    now: i64,
    log: bool,
) -> mush_dice::owod20::Owod20Outcome {
    let opts = Owod20Opts::new(difficulty);
    let outcome = {
        let mut rctx = RollCtx::new(st.rng());
        Owod20Pool.roll(pool, &opts, &mut rctx)
    };
    if log {
        st.push_log(
            observer,
            RollLogEntry {
                who: observer,
                expr: expr.to_string(),
                summary: outcome.summary(),
                difficulty: outcome.difficulty,
                ts: now,
                private: true,
                outcome: outcome.clone(),
            },
        );
    }
    outcome
}

/// Roll every observer present with `subject` against a veil of `rating` and
/// cache each verdict (used when `subject` cloaks or enters a room).
fn refresh_subject(cmd: &WodCmd, ctx: &mut Ctx<'_>, subject: ObjId, rating: i32) {
    let Some(room) = ctx.w.get(subject).and_then(|o| o.location) else {
        return;
    };
    // Gather every observer's sensing dots from the (read-only) world first,
    // then take the mutable state slot for the rolls.
    let observers: Vec<(ObjId, i32, i32, i32)> = ctx
        .w
        .contents_of(room)
        .iter()
        .copied()
        .filter(|&o| o != subject && is_observer(ctx.w, o))
        .map(|o| {
            let (p, a, au) = observer_senses(cmd, ctx.w, o);
            (o, p, a, au)
        })
        .collect();
    if observers.is_empty() {
        return;
    }
    let now = ctx.w.now();
    if let Some(st) = ctx.state_mut::<WodState>() {
        for (o, p, a, au) in observers {
            let pierced = perception_roll(st, o, p, a, au, rating, now);
            st.set_perception(o, subject, pierced);
        }
    }
}

/// Roll `observer` against everyone concealed in its current room and cache
/// each verdict (used when a watcher enters a room / connects).
fn refresh_observer(cmd: &WodCmd, ctx: &mut Ctx<'_>, observer: ObjId) {
    if !is_observer(ctx.w, observer) {
        return;
    }
    let Some(room) = ctx.w.get(observer).and_then(|o| o.location) else {
        return;
    };
    let (perception, awareness, auspex) = observer_senses(cmd, ctx.w, observer);
    // The concealed subjects co-located with `observer`, with their ratings.
    let subjects: Vec<(ObjId, i32)> = {
        let Some(st) = ctx.state::<WodState>() else {
            return;
        };
        ctx.w
            .contents_of(room)
            .iter()
            .copied()
            .filter(|&s| s != observer)
            .filter_map(|s| st.concealed_rating(s).map(|r| (s, r)))
            .collect()
    };
    if subjects.is_empty() {
        return;
    }
    let now = ctx.w.now();
    if let Some(st) = ctx.state_mut::<WodState>() {
        for (s, rating) in subjects {
            let pierced = perception_roll(st, observer, perception, awareness, auspex, rating, now);
            st.set_perception(observer, s, pierced);
        }
    }
}

// ----------------------------------------------------------------------
// M14 — the perception director (per-observer silent projection of an act)
// ----------------------------------------------------------------------

/// The sensory channel an act travels on. [`SenseChannel::Visual`] is noticed
/// with **Perception + Awareness**; [`SenseChannel::Supernatural`] adds the
/// observer's **Auspex** (the preternatural sense) to that pool. Threaded
/// through the adjudicated [`Ruling`](crate::st::Ruling) so a working can be
/// flagged as perceptible only to those who can sense the numinous.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SenseChannel {
    /// An overt, physical act — noticed with mundane Perception + Awareness.
    #[default]
    Visual,
    /// A numinous act — Perception + Awareness + Auspex, and (optionally) gated
    /// behind a `required_sense` so a mundane onlooker cannot notice at all.
    Supernatural,
}

/// How perceptible one act is, and what a successful observer perceives — the
/// ruleset-owned vocabulary M14 routes every **power use** through. Minimal and
/// cross-system-ready: a channel, a witness difficulty, an optional gating
/// sense, and the deterministic line a winner is shown.
#[derive(Clone, Debug)]
pub struct ActPerceptibility {
    /// Whether noticing this act uses the numinous sense pool.
    pub channel: SenseChannel,
    /// The difficulty an onlooker rolls to notice the act.
    pub difficulty: u8,
    /// A sense (a Discipline/trait name, e.g. `"Auspex"`) an observer must have
    /// at least one dot in to notice the act **at all**. `None` ⇒ anyone may try.
    pub required_sense: Option<&'static str>,
    /// The deterministic, in-fiction line a successful observer is shown.
    pub witnessed: String,
}

impl ActPerceptibility {
    /// A perceptibility with an explicit channel/difficulty/gate and line.
    #[must_use]
    pub fn new(
        channel: SenseChannel,
        difficulty: u8,
        required_sense: Option<&'static str>,
        witnessed: impl Into<String>,
    ) -> ActPerceptibility {
        ActPerceptibility {
            channel,
            difficulty,
            required_sense,
            witnessed: witnessed.into(),
        }
    }
}

/// The fallback perceptibility for a power lacking a table entry: an overt
/// (Visual) act of moderate difficulty that anyone present may try to notice.
#[must_use]
pub(crate) fn default_perceptibility(witnessed: impl Into<String>) -> ActPerceptibility {
    ActPerceptibility::new(SenseChannel::Visual, 6, None, witnessed)
}

/// Read `obj`'s sheet through the schema-agnostic default store (the same read
/// [`WodCmd::read`] performs), so the witness core works from either the
/// player path (`&WodCmd`) or the NPC path (`&Dispatch`).
fn read_sheet(dispatch: &Dispatch, w: &World, obj: ObjId) -> Option<SheetDoc> {
    SheetStore::new(dispatch.schemas.default_schema()).get(w, obj)
}

/// **The M14 perception director.** Route one perceptible act through a single
/// per-observer projection policy:
///
/// 1. the **actor** always perceives their own act (`actor_line`);
/// 2. every co-located observer silently rolls to notice it
///    ([`resolve_action_witnesses`] with `perc`);
/// 3. each **winner** is shown `perc.witnessed` — a **loser receives nothing at
///    all** (true silence, no wire artifact, no room narrative, no panel);
/// 4. if `vulgar`, a single obvious-tier line is ALSO emitted to the
///    room-at-large (so a fireball still reads to everyone who is not already a
///    tailored witness);
/// 5. the NPC subset of the winner set is returned (M14 task 2 files intel for
///    exactly these).
///
/// Operates on the low-level `(&Dispatch, &mut World, &mut Outbox)` so both the
/// player power path (`roll_expr`) and the NPC power path (`agent::wod_roll`)
/// share one policy — neither can bypass projection.
pub(crate) fn project_perceptual_act(
    dispatch: &Dispatch,
    w: &mut World,
    out: &mut Outbox,
    actor: ObjId,
    perc: &ActPerceptibility,
    actor_line: &str,
    vulgar: bool,
) -> Vec<ObjId> {
    // (1) The actor always perceives their own act.
    out.sys_to_actor(actor, "wod", actor_line);

    // (2)+(3) Silent per-observer witnesses; each winner gets the tailored line.
    let winners = if perc.witnessed.trim().is_empty() || perc.difficulty == 0 {
        Vec::new()
    } else {
        resolve_action_witnesses(dispatch, w, actor, perc)
    };
    for &win in &winners {
        out.sys_to_actor(win, "wod", perc.witnessed.clone());
    }

    // (4) A blatant act still reads to the whole room: everyone who was not
    // already a tailored witness sees the obvious-tier, in-fiction line (V2) —
    // the `witnessed` description, NEVER the raw mechanical dice string
    // (`actor_line`), which stays Actor-only. Winners already received that same
    // line above (step 3), so they are excluded here to avoid a double delivery;
    // the room is simply not blind to a fireball.
    if vulgar
        && !perc.witnessed.trim().is_empty()
        && let Some(room) = w.get(actor).and_then(|o| o.location)
    {
        let mut exclude = Vec::with_capacity(winners.len() + 1);
        exclude.push(actor);
        exclude.extend_from_slice(&winners);
        out.to_room_from(
            room,
            exclude,
            actor,
            OutputEvent::Narrative {
                kind: NarrativeKind::Emit,
                actor: Some(obj_ref(w, actor)),
                text: RichText::plain(&perc.witnessed),
            },
        );
    }

    // (5) The NPC subset of the winner set. The player power callers
    // (`roll_expr`, `apply_interpret`) — which hold a `Ctx` with the registry —
    // announce these via `WorldEvent::PowerUsed` so a witnessing NPC informant
    // files intel (M14 D6). The director itself carries no registry (it is
    // shared with the registry-less NPC ability seam), so emission lives one
    // level up in the callers that have one.
    winners
        .into_iter()
        .filter(|&o| matches!(w.get(o).map(|x| x.kind), Some(ObjKind::Npc)))
        .collect()
}

/// M13/M14 silent per-perceiver witnesses. Every observer present with `actor`
/// (a Player or NPC other than the actor) rolls on the shared seeded RNG against
/// `perc.difficulty`; the ones with at least one success are returned. The
/// caller pushes a tailored line ONLY to those winners — a loser receives no
/// event at all (A5 true silence, no wire artifact).
///
/// The pool is **Perception + Awareness**, plus the observer's **Auspex** when
/// `perc.channel` is [`SenseChannel::Supernatural`]. When `perc.required_sense`
/// is set, an observer lacking that sense (rating < 1) is skipped **before any
/// roll** — mirroring [`perception_roll`]'s auspex-early-return, so a mundane
/// crowded room advances ZERO RNG and a seeded game stays reproducible.
///
/// Observers are gathered in the stable [`World::contents_of`] order and their
/// sensing dots are read FIRST; the mutable state slot is taken only for the
/// rolls. Each [`sense_roll`] advances the shared RNG, keeping seeded tests
/// deterministic.
pub(crate) fn resolve_action_witnesses(
    dispatch: &Dispatch,
    w: &mut World,
    actor: ObjId,
    perc: &ActPerceptibility,
) -> Vec<ObjId> {
    let Some(room) = w.get(actor).and_then(|o| o.location) else {
        return Vec::new();
    };
    // Snapshot the room's contents (releases the `&World` borrow), then read each
    // observer's sensing dots off the sheet before taking the mutable state slot.
    let ids: Vec<ObjId> = w.contents_of(room).to_vec();
    let observers: Vec<(ObjId, i32)> = ids
        .into_iter()
        .filter(|&o| o != actor && is_observer(w, o))
        .filter_map(|o| {
            let doc = read_sheet(dispatch, w, o);
            // required_sense gate: no sense ⇒ cannot notice, and no roll is made
            // (the mundane-room-advances-no-RNG invariant).
            if let Some(sense) = perc.required_sense {
                let rating = doc.as_ref().map_or(0, |d| discipline_rating(d, sense));
                if rating < 1 {
                    return None;
                }
            }
            let (perception, awareness, auspex) = doc.as_ref().map_or((0, 0, 0), |d| {
                (
                    d.number("perception").unwrap_or(0) as i32,
                    d.number("awareness").unwrap_or(0) as i32,
                    discipline_rating(d, "Auspex"),
                )
            });
            let mut pool = perception + awareness;
            if perc.channel == SenseChannel::Supernatural {
                pool += auspex;
            }
            Some((o, pool))
        })
        .collect();
    if observers.is_empty() {
        return Vec::new();
    }
    let now = w.now();
    let mut winners = Vec::new();
    if let Some(st) = w.module_state_mut::<WodState>() {
        for (o, pool) in observers {
            let expr = format!(
                "perception+awareness vs witness difficulty {}",
                perc.difficulty
            );
            // `log = false`: leave NO per-observer roll-log entry. A witness roll
            // that logged would let a non-perceiving observer read the act (and
            // its witness difficulty) out of their own `+roll/last` — the V1 leak.
            let outcome = sense_roll(st, o, pool, perc.difficulty, &expr, now, false);
            if outcome.successes() > 0 {
                winners.push(o);
            }
        }
    }
    winners
}

/// Dispatch a resolved NPC pool to the power it names (if any), returning the
/// [`ActPerceptibility`] to project it through. A discipline/gift named in the
/// pool, or a mage working (the `arete` field), routes through the director;
/// a mundane pool (`strength+brawl`) returns `None` and stays room-visible.
#[must_use]
pub(crate) fn power_perceptibility_for_pool(
    resolved: &ResolvedPool,
    actor_name: &str,
) -> Option<ActPerceptibility> {
    // A Discipline named in the pool (its map key rides a `PoolTerm::label`).
    for term in &resolved.terms {
        if let Some(p) = crate::splats::vampire::discipline_perceptibility(&term.label, actor_name) {
            return Some(p);
        }
    }
    // A Gift named in the pool.
    for term in &resolved.terms {
        if let Some(p) = crate::splats::werewolf::gift_perceptibility(&term.label, actor_name) {
            return Some(p);
        }
    }
    // A mage working rolls the Arete pool.
    if resolved
        .trait_fields
        .iter()
        .any(|f| f.eq_ignore_ascii_case("arete"))
    {
        return Some(crate::splats::mage::sphere_perceptibility(actor_name));
    }
    None
}

// ----------------------------------------------------------------------
// +obfuscate command (D6)
// ----------------------------------------------------------------------

/// `+obfuscate` / `+obfuscate on` / `+obfuscate off` — cloak with, or drop, the
/// Discipline of concealment. Self-gated to a character with Obfuscate ≥ 1.
fn obfuscate_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let doc = cmd
        .read(ctx.w, ctx.actor)
        .ok_or_else(|| CmdError::msg("You have no sheet."))?;
    let obf = discipline_rating(&doc, "Obfuscate");
    if obf < 1 {
        return Err(CmdError::msg(
            "You do not know the Discipline of Obfuscate.",
        ));
    }
    let actor = ctx.actor;
    let arg = ctx.args.raw().trim().to_ascii_lowercase();
    match arg.as_str() {
        "on" | "hide" | "cloak" => {
            if let Some(st) = ctx.state_mut::<WodState>() {
                st.conceal(actor, obf);
            }
            // Roll everyone present now to notice (once).
            refresh_subject(cmd, ctx, actor, obf);
            ctx.out.sys_to_actor(
                actor,
                "wod",
                "You draw the shadows about yourself and fade from sight. \
                 (Anyone here with a sharp enough eye may still notice you.)",
            );
            Ok(())
        }
        "off" | "show" | "reveal" | "drop" => {
            let was = ctx
                .state_mut::<WodState>()
                .is_some_and(|st| st.reveal(actor));
            let msg = if was {
                "You let the shadows fall away and step back into view."
            } else {
                "You were not concealed."
            };
            ctx.out.sys_to_actor(actor, "wod", msg);
            Ok(())
        }
        "" => {
            let concealed = ctx
                .state::<WodState>()
                .is_some_and(|st| st.is_concealed(actor));
            let msg = if concealed {
                "You are currently Obfuscated (+obfuscate off to reappear)."
            } else {
                "You are in plain sight (+obfuscate on to conceal yourself)."
            };
            ctx.out.sys_to_actor(actor, "wod", msg);
            Ok(())
        }
        other => Err(CmdError::msg(format!(
            "Usage: +obfuscate on|off  (got {other:?})"
        ))),
    }
}

// ----------------------------------------------------------------------
// Registration
// ----------------------------------------------------------------------

/// Register the perception hook, the presence observer, and `+obfuscate`.
pub(crate) fn register(reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
    reg.hook(HookPoint::Perceive, Arc::new(PerceptionHook));
    let observer: Arc<dyn WorldObserver> = Arc::new(PerceptionObserver {
        cmd: Arc::clone(&cmd),
    });
    reg.subscribe(WorldEventKind::Moved, Arc::clone(&observer));
    reg.subscribe(WorldEventKind::Connected, Arc::clone(&observer));
    reg.subscribe(WorldEventKind::Disconnected, observer);

    reg.command(
        CommandSpec {
            name: "+obfuscate".into(),
            shape: ArgShape::Raw,
            help: "+obfuscate  --  Show whether you are concealed.\n\
                   +obfuscate on  --  Cloak yourself with Obfuscate; everyone present rolls \
                   (Perception + Awareness + Auspex) once to notice you.\n\
                   +obfuscate off  --  Step back into view. Requires the Obfuscate Discipline.\n\
                   While cloaked you emit true silence to those who fail to notice: they do \
                   not see you in the room, cannot examine you, and never hear you speak."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| obfuscate_cmd(&cmd, ctx))
        },
    )?;
    Ok(())
}
