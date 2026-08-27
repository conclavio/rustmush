//! The core command set: speech, looking, movement, item handling,
//! `@`-building, and info commands — all registered through the one
//! [`Registry`](crate::cmd::Registry) and dispatched per treaty ruling 5.

use mush_proto::{NarrativeKind, OutputEvent, RichText};

use crate::cmd::{Registry, RegistryError};
use crate::obj::ObjId;
use crate::world::World;

pub mod building;
pub mod info;
pub mod items;
pub mod look;
pub mod movement;
pub mod speech;

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
