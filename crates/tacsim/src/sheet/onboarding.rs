//! The onboarding gate: hold a freshly-created, not-yet-approved character to a
//! chargen command set until staff approve it (the `chargen_required` flow),
//! greet them on connect, place them in the chargen area, and lift the gate —
//! moving them to the live start room — on approval.
//!
//! Ported near-verbatim from `ruleset-wod`'s, over one [`SheetStore`] instead of
//! a template dispatch, and with the two things the game does not have removed:
//! `+spec` (no specialties) and the connect-time drop into the wizard (no
//! wizard). The three seams are the same:
//!
//! - a [`CommandGate`] ([`OnboardingGate`]) consulted in dispatch, before the
//!   comsys alias stage, so channel *speech* is gated too;
//! - a `Connected` [`EventHook`] ([`OnboardingHook`]) that greets a gated player
//!   and points at `+chargen`;
//! - an `ObjCreated` [`WorldObserver`] ([`OnboardingSpawn`]) that places a new
//!   Player in `start_room_unapproved`.
//!
//! Everything here is **inert unless `chargen_required` is on**, which is the
//! default — a server that never configures `[onboarding]` is byte-identical to
//! one built without this file.

use std::sync::Arc;

use mush_core::{
    CommandGate, Ctx, EventHook, ObjId, ObjKind, OnboardingConfig, Registry, Role, World,
    WorldEvent, WorldObserver, perm,
};
use mush_ruleset::schema::SheetSchema;
use mush_ruleset::{ApprovalState, SheetStore};

/// Whether `verb` is on the onboarding allowlist — the commands an unapproved,
/// gated character may still run. Everything else (exit names, `home`, building,
/// rolling, movement onto the grid) is refused. The gate runs **before** the
/// comsys alias stage, so channel speech (a per-player alias like `pub Hello`)
/// reaches it as its alias verb, which is not listed here, and is refused. The
/// channel *management* verbs stay listed so a waiting player can still
/// `comlist`/`addcom`.
#[must_use]
pub fn is_onboarding_verb(verb: &str) -> bool {
    const ALLOWED: &[&str] = &[
        // Perception + basic room roleplay in the chargen area.
        "look", "l", "say", "pose", "semipose", "think", // Help + who + leaving.
        "help", "+help", "who", "quit", // Character generation.
        "+chargen", "+stat", "+sheet",
        // Channel management (join/list); speaking on a channel is gated.
        "addcom", "delcom", "comlist", "comtitle", "clist",
    ];
    let verb = verb.to_ascii_lowercase();
    ALLOWED.iter().any(|allowed| *allowed == verb)
}

/// The message shown when the gate refuses a command.
fn refusal() -> String {
    "You must finish creating your character and have it approved before you can do that. \
     Type +chargen to keep building; look, say, pose, and help still work while you wait."
        .to_string()
}

/// Whether `actor`'s sheet is Approved (an absent sheet is *not* approved).
fn is_approved(schema: &Arc<SheetSchema>, w: &World, actor: ObjId) -> bool {
    SheetStore::new(Arc::clone(schema))
        .get(w, actor)
        .is_some_and(|doc| matches!(doc.approved, ApprovalState::Approved { .. }))
}

/// Whether the onboarding restriction applies to `actor` right now: gating is
/// on, `actor` is a plain (non-staff) Player, and their sheet is not Approved.
fn gated(cfg: &OnboardingConfig, schema: &Arc<SheetSchema>, w: &World, actor: ObjId) -> bool {
    cfg.chargen_required
        && w.get(actor).is_some_and(|o| o.kind == ObjKind::Player)
        && !perm::has_role(w, actor, Role::Builder)
        && !is_approved(schema, w, actor)
}

/// The dispatch-level gate (treaty ruling 5's pre-command seam).
pub struct OnboardingGate {
    cfg: OnboardingConfig,
    schema: Arc<SheetSchema>,
}

impl OnboardingGate {
    /// The gate for `cfg` over `schema`.
    #[must_use]
    pub fn new(cfg: OnboardingConfig, schema: Arc<SheetSchema>) -> OnboardingGate {
        OnboardingGate { cfg, schema }
    }
}

impl CommandGate for OnboardingGate {
    fn check(
        &self,
        w: &World,
        _reg: &Registry,
        actor: ObjId,
        verb: &str,
        line: &str,
    ) -> Result<(), String> {
        if !gated(&self.cfg, &self.schema, w, actor) {
            return Ok(());
        }
        // Movement onto the live grid is the gate's central block. The verb
        // allowlist alone does not imply "no exit will match": dispatch matches
        // exits (room -> zone -> global exits on Limbo) after this gate and
        // against the whole line, so a global or zone exit whose name or alias
        // collides with an allowlisted verb (`l`, `look`, `say`, ...) would slip
        // through. Refuse any line that resolves to an exit.
        if mush_core::commands::movement::find_exit(w, actor, line).is_some() {
            return Err(refusal());
        }
        if !is_onboarding_verb(verb) {
            return Err(refusal());
        }
        Ok(())
    }
}

/// The `Connected` hook: greet a gated character and point them at `+chargen`.
///
/// The WoD twin pushed its guided wizard here. There is no wizard — the flow is
/// three commands — so this stays a greeting, and it still refuses to paint over
/// a live input context (a reconnect into an open prompt).
pub struct OnboardingHook {
    cfg: OnboardingConfig,
    schema: Arc<SheetSchema>,
}

impl OnboardingHook {
    /// The hook for `cfg` over `schema`.
    #[must_use]
    pub fn new(cfg: OnboardingConfig, schema: Arc<SheetSchema>) -> OnboardingHook {
        OnboardingHook { cfg, schema }
    }
}

impl EventHook for OnboardingHook {
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        if !gated(&self.cfg, &self.schema, ctx.w, actor) {
            return;
        }
        if ctx.w.input_depth(actor) > 0 {
            return;
        }
        // A submitted (Pending) sheet is locked awaiting a storyteller — say so
        // rather than inviting them back into chargen.
        let approved = SheetStore::new(Arc::clone(&self.schema))
            .get(ctx.w, actor)
            .map(|doc| doc.approved);
        if let Some(ApprovalState::Pending(job)) = approved {
            ctx.out.info(
                actor,
                format!(
                    "Your character is submitted and awaiting approval (job {job}). \
                     A storyteller will review it soon."
                ),
            );
            return;
        }
        ctx.out.info(
            actor,
            "Welcome! Type +chargen to finish creating your character: spend your pilot \
             ratings with +stat, then +chargen/submit.",
        );
    }
}

/// The `ObjCreated` observer: place a new unapproved Player in the chargen area.
pub struct OnboardingSpawn {
    cfg: OnboardingConfig,
}

impl OnboardingSpawn {
    /// The observer for `cfg`.
    #[must_use]
    pub fn new(cfg: OnboardingConfig) -> OnboardingSpawn {
        OnboardingSpawn { cfg }
    }
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
/// [`pilot.chargen`](super::chargen::CHARGEN_HANDLER) job handler after the
/// sheet is stamped Approved. A no-op when `chargen_required` is off.
pub fn on_approved(cfg: &OnboardingConfig, ctx: &mut Ctx<'_>, target: ObjId) {
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
    // Show them their new surroundings. **Render the room directly** rather than
    // re-dispatching `"look"` (08j S2): a re-dispatch from inside a handler has
    // no lens to hand `mush_core::dispatch` but a detached one, so the whole
    // perception chain would run against an explicitly unavailable typed reader.
    // `Ctx::show_room` is the one room-render path and carries this command's
    // live snapshot. The empty-input-stack guard stays: painting a room over an
    // open prompt is wrong even when it cannot be swallowed as body text.
    if moved && ctx.w.input_depth(target) == 0 {
        ctx.show_room(target, dest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_allowlist_carries_chargen_and_not_the_grid() {
        for verb in ["+chargen", "+stat", "+sheet", "look", "say", "help", "quit"] {
            assert!(is_onboarding_verb(verb), "{verb} should be allowed");
        }
        assert!(is_onboarding_verb("LOOK"), "the test is case-insensitive");
        for verb in ["+roll", "@dig", "home", "page", "+spec"] {
            assert!(!is_onboarding_verb(verb), "{verb} should be gated");
        }
    }
}
