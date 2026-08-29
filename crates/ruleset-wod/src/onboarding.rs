//! The onboarding gate: hold a freshly-created, not-yet-approved character to
//! a chargen command set until staff approve it (the `chargen_required` flow),
//! drop them into the wizard on connect, place them in the chargen area, and
//! lift the gate — moving them to the live start room — on approval.
//!
//! Everything here is **inert unless `chargen_required` is on** (the default is
//! off, for byte-identical back-compat). It hangs on three seams:
//!
//! - a [`CommandGate`] ([`OnboardingGate`]) consulted in dispatch (before the
//!   comsys alias stage, so channel broadcast is gated too): an unapproved
//!   player may run only the onboarding command set ([`is_onboarding_verb`]) —
//!   chargen, look, room speech, help — and nothing that moves them onto,
//!   broadcasts across, or builds on the live grid; staff and approved
//!   characters pass freely;
//! - a `Connected` [`EventHook`] ([`OnboardingHook`]) that greets an unapproved
//!   player and pushes the [wizard](crate::wizard) (unless one is already
//!   pending or a context is open);
//! - an `ObjCreated` [`WorldObserver`] ([`OnboardingSpawn`]) that places a new
//!   Player in `start_room_unapproved`.
//!
//! On approval the `wod.chargen` job handler calls [`on_approved`], which moves
//! the character to `start_room` and re-renders it; the gate then passes on its
//! own (the sheet is Approved), so nothing else has to be un-wired.

use std::sync::Arc;

use mush_core::{
    CommandGate, Ctx, EventHook, LIMBO, ObjId, ObjKind, Role, World, WorldEvent, WorldObserver,
    perm,
};
use mush_ruleset::ApprovalState;

use crate::WodCmd;
use crate::splats::Dispatch;

/// The onboarding configuration (from `mushd.toml`'s `[onboarding]`), carried
/// by the ruleset and shared by the gate / hook / observer / approval move.
///
/// All-default (`chargen_required = false`, both rooms `#0`/Limbo) is a no-op:
/// the gate always passes, the hook never fires, the spawn move is a no-op, and
/// approval moves no one — exactly the pre-onboarding behaviour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnboardingConfig {
    /// When set, unapproved characters are gated to the chargen command set
    /// and dropped into the wizard on connect.
    pub chargen_required: bool,
    /// The live start room an approved character is moved to.
    pub start_room: ObjId,
    /// The restricted chargen area a fresh, unapproved character is placed in.
    pub start_room_unapproved: ObjId,
}

impl Default for OnboardingConfig {
    fn default() -> OnboardingConfig {
        OnboardingConfig {
            chargen_required: false,
            start_room: LIMBO,
            start_room_unapproved: LIMBO,
        }
    }
}

/// Whether `verb` is on the onboarding allowlist — the commands an unapproved,
/// gated character may still run. Everything else (exit names, `home`,
/// building, rolling, movement onto the grid) is refused. The gate now runs
/// **before** the comsys alias stage, so channel *speech* (a per-player alias
/// like `pub Hello`) is refused too — it reaches the gate as its alias verb,
/// which is not listed here. The channel *management* verbs stay listed so a
/// waiting player can still `comlist`/`addcom`; they cannot speak on a channel
/// until approved.
#[must_use]
pub fn is_onboarding_verb(verb: &str) -> bool {
    const ALLOWED: &[&str] = &[
        // Perception + basic room roleplay in the chargen area.
        "look", "l", "say", "pose", "semipose", "think", // Help + who + leaving.
        "help", "+help", "who", "quit", // Character generation.
        "+chargen", "+stat", "+spec", "+sheet",
        // Channel management (join/list); speaking on a channel is gated.
        "addcom", "delcom", "comlist", "comtitle", "clist",
    ];
    let v = verb.to_ascii_lowercase();
    ALLOWED.iter().any(|a| *a == v)
}

/// The message shown when the gate refuses a command.
fn refusal() -> String {
    "You must finish creating your character and have it approved before you can do that. \
     Type +chargen/wizard to keep building (or +chargen for the classic tools); look, say, \
     pose, and help still work while you wait."
        .to_string()
}

/// Whether `actor`'s sheet is Approved (an absent sheet is *not* approved).
fn is_approved(dispatch: &Dispatch, w: &World, actor: ObjId) -> bool {
    dispatch
        .read(w, actor)
        .is_some_and(|d| matches!(d.approved, ApprovalState::Approved { .. }))
}

/// Whether the onboarding restriction applies to `actor` right now: gating is
/// on, `actor` is a plain (non-staff) Player, and their sheet is not Approved.
fn gated(cfg: &OnboardingConfig, dispatch: &Dispatch, w: &World, actor: ObjId) -> bool {
    cfg.chargen_required
        && w.get(actor).is_some_and(|o| o.kind == ObjKind::Player)
        && !perm::has_role(w, actor, Role::Builder)
        && !is_approved(dispatch, w, actor)
}

/// The dispatch-level gate (treaty ruling 5's new pre-command seam).
pub(crate) struct OnboardingGate {
    pub(crate) cfg: OnboardingConfig,
    pub(crate) dispatch: Arc<Dispatch>,
}

impl CommandGate for OnboardingGate {
    fn check(&self, w: &World, actor: ObjId, verb: &str, line: &str) -> Result<(), String> {
        if !gated(&self.cfg, &self.dispatch, w, actor) {
            return Ok(());
        }
        // Movement onto the live grid is the gate's central block. The verb
        // allowlist alone does not imply "no exit will match": dispatch matches
        // exits (room → zone → global exits on Limbo) after this gate and
        // against the whole line, so a global/zone exit whose name or alias
        // collides with an allowlisted verb (`l`, `look`, `say`, …) would slip
        // through. Refuse any line that resolves to an exit — mirroring the
        // read-only gate's `find_exit` guard in the world task.
        if mush_core::commands::movement::find_exit(w, actor, line).is_some() {
            return Err(refusal());
        }
        if !is_onboarding_verb(verb) {
            return Err(refusal());
        }
        Ok(())
    }
}

/// The `Connected` hook: greet a gated character and drop them into the wizard.
pub(crate) struct OnboardingHook {
    pub(crate) cfg: OnboardingConfig,
    pub(crate) cmd: Arc<WodCmd>,
}

impl EventHook for OnboardingHook {
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        if !gated(&self.cfg, &self.cmd.dispatch, ctx.w, actor) {
            return;
        }
        // Don't stack a second wizard over a live input context (e.g. a
        // reconnect into a wizard that survived the drop).
        if ctx.w.input_depth(actor) > 0 {
            return;
        }
        // A submitted (Pending) sheet is locked awaiting a storyteller — say
        // so rather than re-opening chargen.
        if let Some(ApprovalState::Pending(job)) = self.cmd.read(ctx.w, actor).map(|d| d.approved) {
            ctx.out.info(
                actor,
                format!(
                    "Your character is submitted and awaiting approval (job {job}). \
                     A storyteller will review it soon."
                ),
            );
            return;
        }
        ctx.out
            .info(actor, "Welcome! Let's finish creating your character.");
        if let Err(e) = crate::wizard::start_wizard(Arc::clone(&self.cmd), ctx) {
            ctx.out.error(actor, e.to_string());
        }
    }
}

/// The `ObjCreated` observer: place a new unapproved Player in the chargen area.
pub(crate) struct OnboardingSpawn {
    pub(crate) cfg: OnboardingConfig,
}

impl WorldObserver for OnboardingSpawn {
    fn on_event(&self, ctx: &mut Ctx<'_>, ev: &WorldEvent) {
        if !self.cfg.chargen_required {
            return;
        }
        let WorldEvent::ObjCreated { obj, kind, .. } = ev else {
            return;
        };
        if *kind != ObjKind::Player {
            return;
        }
        let dest = self.cfg.start_room_unapproved;
        if ctx.w.get(dest).is_none() {
            return;
        }
        if ctx.w.get(*obj).and_then(|o| o.location) == Some(dest) {
            return;
        }
        let _ = ctx.w.move_to(*obj, dest);
    }
}

/// Lift the gate on approval: move `target` to the live start room (if
/// configured and not already there) and re-render it. Called by the
/// `wod.chargen` job handler after the sheet is stamped Approved. A no-op when
/// `chargen_required` is off.
pub(crate) fn on_approved(cfg: &OnboardingConfig, ctx: &mut Ctx<'_>, target: ObjId) {
    if !cfg.chargen_required {
        return;
    }
    let dest = cfg.start_room;
    let at_dest = ctx.w.get(target).and_then(|o| o.location) == Some(dest);
    let dest_exists = ctx.w.get(dest).is_some();
    // Misconfiguration guard: `start_room` points at a room that no longer
    // exists and the character is not already there, so the post-approval move
    // cannot happen — they stay in the (possibly exit-less) chargen area even
    // though we are about to tell them they have full access. Surface it to
    // staff via the log rather than stranding them silently; the sheet is
    // already stamped Approved by the caller, so we cannot un-approve here.
    if !dest_exists && !at_dest {
        tracing::warn!(
            %target,
            start_room = %dest,
            "onboarding: start_room does not exist; approved character was not relocated \
             out of the chargen area — check [onboarding].start_room"
        );
    }
    let moved = dest_exists && !at_dest && ctx.w.move_to(target, dest).is_ok();
    ctx.out.info(
        target,
        "Your character has been approved — welcome to the game! You now have full access.",
    );
    // Show them their new surroundings (they're approved now, so the gate
    // waves `look` through). Guard on an empty input stack: dispatching a
    // command line into an open input context would feed "look" to it as body
    // text (the class of bug the connect-time auto-look skip avoids). A Pending
    // gated character cannot open a context today, so this is defensive.
    if moved && ctx.w.input_depth(target) == 0 {
        mush_core::dispatch(ctx.reg, ctx.w, target, "look", ctx.out);
    }
}
