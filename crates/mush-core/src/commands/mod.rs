//! The core command set: speech, looking, movement, item handling,
//! `@`-building, and info commands — all registered through the one
//! [`Registry`] and dispatched per treaty ruling 5.

use mush_proto::{NarrativeKind, OutputEvent, RichText};

use crate::cmd::{Registry, RegistryError};
use crate::flags::{Flags, Powers};
use crate::module::{HookPoint, perceives_via};
use crate::obj::{ObjId, ObjKind};
use crate::output::{Outbox, room_view};
use crate::perm;
use crate::world::World;

pub mod admin;
pub mod building;
pub mod info;
pub mod items;
pub mod look;
pub mod macros;
pub mod movement;
pub mod speech;

pub use admin::{event_subject, ignore_list, is_ignoring, narrative_actor};
pub use speech::{can_emit, sees_spoof_source, spoof_for_listener};

/// Register every core command into `reg`. Called once at boot (and by
/// tests); collisions among core commands are a programming error surfaced
/// as [`RegistryError`].
pub fn register_core_commands(reg: &mut Registry) -> Result<(), RegistryError> {
    speech::register(reg)?;
    look::register(reg)?;
    movement::register(reg)?;
    items::register(reg)?;
    building::register(reg)?;
    info::register(reg)?;
    admin::register(reg)?;
    // The kernel's `@macro` and its alias stage (16b cut AD.1): registered
    // here, before any module composes, so the macro stage is the FIRST
    // alias stage on every composition.
    macros::register(reg)?;
    crate::behavior::register(reg)?;
    Ok(())
}

/// Inherited attribute as owned text (`None` for missing or JSON attrs).
pub(crate) fn attr_text(w: &World, id: ObjId, name: &str) -> Option<String> {
    w.attr(id, name)
        .and_then(|a| a.as_text())
        .map(str::to_string)
}

/// An unattributed narrative line (movement echoes, take/drop spam, …).
pub(crate) fn narrative_emit(text: RichText) -> OutputEvent {
    OutputEvent::Narrative {
        kind: NarrativeKind::Emit,
        actor: None,
        text,
    }
}

/// Show a room render to `looker`: the structured [`RoomView`] event plus
/// any `LookRendered` hook lines (weather ambience, scene banners). The
/// one room-render path — `look`, movement arrival, and `@teleport` all
/// come through here so module lines appear everywhere a room does.
///
/// [`RoomView`]: mush_proto::RoomView
pub(crate) fn show_room(
    reg: &Registry,
    w: &World,
    looker: ObjId,
    room: ObjId,
    out: &mut Outbox,
    lens: crate::action_state::TypedStateLens<'_>,
) {
    if let Some(rv) = room_view(reg, w, looker, room, lens) {
        out.to_actor(looker, OutputEvent::Room(rv));
        for line in crate::module::look_rendered_lines(reg, w, looker, room, lens) {
            out.to_actor(looker, OutputEvent::Sys(line));
        }
    }
}

/// Whether `target` is visibly co-located with `observer` in `observer_loc`,
/// honoring `Flags::GOING`, `Flags::DARK`, uncontrolled `DARK` rooms, and
/// `HookPoint::Perceive` (P2-CORE-13).
///
/// Intended as a filter for [`crate::matchit::match_object_visible`] so hidden
/// or cloaked targets cannot leak existence via exact match, prefix ambiguity,
/// ordinal shifts, `#dbref`, or `*name`.
pub(super) fn target_visible_here(
    reg: &Registry,
    w: &World,
    observer: ObjId,
    target: ObjId,
    observer_loc: Option<ObjId>,
    lens: crate::action_state::TypedStateLens<'_>,
) -> bool {
    let Some(o) = w.get(target) else {
        return false;
    };
    if o.flags.contains(Flags::GOING) {
        return false;
    }
    if target == observer {
        return true;
    }
    let Some(loc) = observer_loc else {
        return false;
    };
    if o.location != Some(loc) {
        return false;
    }
    let see_hidden = perm::has_power(w, observer, Powers::SEE_HIDDEN);
    let controls_target = perm::controls(w, observer, target);
    if o.flags.contains(Flags::DARK) && !see_hidden && !controls_target {
        return false;
    }
    if o.kind != ObjKind::Exit && !see_hidden && !controls_target {
        let in_dark_room = w.get(loc).is_some_and(|r| r.flags.contains(Flags::DARK))
            && !perm::controls(w, observer, loc);
        if in_dark_room {
            return false;
        }
    }
    perceives_via(reg.hooks(HookPoint::Perceive), w, observer, target, lens)
}
