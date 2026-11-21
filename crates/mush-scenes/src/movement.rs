//! Pure selection of scene transcript events for a physical actor move.
//!
//! The `Moved` observer appends what this selects; it holds no capture limits.

use mush_core::ObjId;

use crate::{SceneEvent, SceneEventKind, SceneRecord, SceneStatus, SceneUid, ScenesState};

/// One original scene event and the ledger to which it belongs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlannedSceneEvent {
    scene: SceneUid,
    event: SceneEvent,
}

impl PlannedSceneEvent {
    #[cfg(test)]
    pub(crate) const fn scene(&self) -> SceneUid {
        self.scene
    }

    #[cfg(test)]
    pub(crate) const fn event(&self) -> &SceneEvent {
        &self.event
    }

    pub(crate) fn into_parts(self) -> (SceneUid, SceneEvent) {
        (self.scene, self.event)
    }
}

/// The events a move from `from` to `to` appends, destination entry first.
///
/// Concealed (`Flags::DARK`) actors who are not members of a scene are skipped
/// for that scene so passive movement never leaks hidden visitors (such as DARK
/// staff) into scene transcripts (EXT-P2-13). Scene members keep their existing
/// recording behavior.
pub(crate) fn events_for_move(
    state: &ScenesState,
    actor: ObjId,
    actor_dark: bool,
    from: Option<ObjId>,
    to: ObjId,
    at: i64,
) -> Vec<PlannedSceneEvent> {
    materialize(select(state, actor, actor_dark, from, to), actor, at)
}

#[cfg(test)]
pub(crate) fn legacy_events(
    state: &ScenesState,
    actor: ObjId,
    from: Option<ObjId>,
    to: ObjId,
    at: i64,
) -> Vec<PlannedSceneEvent> {
    events_for_move(state, actor, false, from, to, at)
}

fn select(
    state: &ScenesState,
    actor: ObjId,
    actor_dark: bool,
    from: Option<ObjId>,
    to: ObjId,
) -> [Option<(&SceneRecord, SceneEventKind)>; 2] {
    if from == Some(to) {
        return [None, None];
    }
    // Preserve the existing lookup exactly: the first holding scene masks any
    // later scene at the same room, even when the first is paused.
    let active = |room| {
        state
            .active_scene_at(room)
            .filter(|scene| scene.status == SceneStatus::Active)
            .filter(|scene| !actor_dark || scene.members().any(|m| m == actor))
    };
    [
        active(to).map(|scene| (scene, SceneEventKind::Joined)),
        from.and_then(active)
            .map(|scene| (scene, SceneEventKind::Left)),
    ]
}

fn materialize(
    selected: [Option<(&SceneRecord, SceneEventKind)>; 2],
    actor: ObjId,
    at: i64,
) -> Vec<PlannedSceneEvent> {
    selected
        .into_iter()
        .flatten()
        .map(|(scene, kind)| PlannedSceneEvent {
            scene: scene.id,
            event: SceneEvent::new(scene, at, kind, Some(actor), serde_json::json!({})),
        })
        .collect()
}

#[cfg(test)]
#[path = "movement_tests.rs"]
mod tests;
