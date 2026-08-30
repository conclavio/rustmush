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
    ArgShape, CmdError, CommandSpec, Ctx, EventHook, HookPoint, ModError, ObjId, ObjKind, Perceive,
    Registrar, World, WorldEvent, WorldEventKind, WorldObserver,
};
use mush_dice::RollCtx;
use mush_dice::owod20::{Owod20Opts, Owod20Pool};
use mush_dice::pool::PoolMechanic;

use crate::WodCmd;
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
    sense_roll(st, observer, pool, difficulty, &expr, now).successes() > 0
}

/// The headless sensing-roll core: roll `pool` dice at `difficulty` on the
/// shared seeded RNG, record a **private** roll-log entry (staff-auditable), and
/// return the full [`Owod20Outcome`]. No sheet touch, no room output — the only
/// side effects are the RNG draw and the log. Shared by the M11 concealment
/// pierce ([`perception_roll`]) and the M13 silent-witness resolution
/// ([`resolve_action_witnesses`]); every call advances the shared RNG, which is
/// what keeps seeded tests deterministic.
pub(crate) fn sense_roll(
    st: &mut WodState,
    observer: ObjId,
    pool: i32,
    difficulty: u8,
    expr: &str,
    now: i64,
) -> mush_dice::owod20::Owod20Outcome {
    let opts = Owod20Opts::new(difficulty);
    let outcome = {
        let mut rctx = RollCtx::new(st.rng());
        Owod20Pool.roll(pool, &opts, &mut rctx)
    };
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

/// M13 silent per-perceiver witnesses. Every observer present with `actor` (a
/// Player or NPC other than the actor) rolls **Perception + Awareness** on the
/// shared seeded RNG against `witness_difficulty`; the ones with at least one
/// success are returned. The caller pushes a tailored line ONLY to those
/// winners — a loser receives no event at all (A5 true silence, no wire
/// artifact).
///
/// Observers are gathered in the stable [`World::contents_of`] order and their
/// sensing dots are read from the read-only `&World` FIRST; the mutable state
/// slot is taken only for the rolls, so the borrow discipline matches
/// [`refresh_subject`]. Each [`sense_roll`] advances the shared RNG, keeping
/// seeded tests deterministic.
pub(crate) fn resolve_action_witnesses(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    witness_difficulty: u8,
) -> Vec<ObjId> {
    let Some(room) = ctx.w.get(actor).and_then(|o| o.location) else {
        return Vec::new();
    };
    // Read every observer's Perception + Awareness from the read-only world
    // first (auspex is irrelevant to noticing a mundane act), then take the
    // mutable state slot for the rolls.
    let observers: Vec<(ObjId, i32)> = ctx
        .w
        .contents_of(room)
        .iter()
        .copied()
        .filter(|&o| o != actor && is_observer(ctx.w, o))
        .map(|o| {
            let (perception, awareness, _auspex) = observer_senses(cmd, ctx.w, o);
            (o, perception + awareness)
        })
        .collect();
    if observers.is_empty() {
        return Vec::new();
    }
    let now = ctx.w.now();
    let mut winners = Vec::new();
    if let Some(st) = ctx.state_mut::<WodState>() {
        for (o, pool) in observers {
            let expr = format!("perception+awareness vs witness difficulty {witness_difficulty}");
            let outcome = sense_roll(st, o, pool, witness_difficulty, &expr, now);
            if outcome.successes() > 0 {
                winners.push(o);
            }
        }
    }
    winners
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
