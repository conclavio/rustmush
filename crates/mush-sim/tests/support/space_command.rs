use mush_core::{Audience, ObjId};
use mush_proto::OutputEvent;
use mush_testkit::authoritative::AcknowledgedWorld;

/// Only the command migrated in this cut opts into the explicit coordinator.
/// Other test commands retain their ordinary TestWorld execution semantics.
pub fn run(
    world: &mut AcknowledgedWorld,
    actor: ObjId,
    line: &str,
) -> Vec<(Audience, OutputEvent)> {
    if line.starts_with("@spatial/space ") {
        world.run_and_commit_one(actor, line, "sim", mush_core::physical_transition::ACTION)
    } else {
        world.run(actor, line)
    }
}
