//! The `scenes` panel feed, its query, and the room-ambient glance
//! (interface plan I1, D-I-4).
//!
//! Three read-only projections of one [`SceneRecord`], from one place:
//!
//! - [`glance`] — what fits on a status line (`RoomView.ambient["scene"]`),
//!   contributed by [`SceneAmbient`] for the room the scene is anchored to;
//! - [`keyframe`] — the full scene screen payload, answered by
//!   [`ScenesQuery`] for the actor's own active scene and **pushed to every
//!   member** by [`publish`] after each lifecycle command (start, next,
//!   pause, resume, release, end) has mutated and persisted the record;
//! - `null`, pushed by [`publish_none`] to a member released mid-scene so
//!   their screen clears.
//!
//! Nothing here is authority: the record is the scene, this is its picture.

use mush_core::output::obj_ref;
use mush_core::{Ctx, EventHook, HookReadContext, ObjId, QueryHandler, World};
use mush_proto::{OutputEvent, PanelFrameKind, RichText, SmolStr};

use crate::data::{SceneRecord, ScenesState};

/// The panel feed and query name.
pub const FEED: &str = "scenes";
/// The `RoomView.ambient` key the glance rides under.
pub const AMBIENT_KEY: &str = "scene";

/// The status-line glance: id, title, kind, mode, status, round, turn, and
/// whose turn it is.
pub(crate) fn glance(w: &World, rec: &SceneRecord) -> serde_json::Value {
    serde_json::json!({
        "id": rec.id.0,
        "title": rec.title,
        "kind": rec.kind,
        "mode": rec.mode,
        "status": rec.status,
        "round": rec.round,
        "turn": rec.turn,
        "current_actor": rec.current_actor().map(|a| obj_ref(w, a)),
    })
}

/// The full scene-screen keyframe: the glance plus membership, anchor,
/// facilitator, clocks.
pub(crate) fn keyframe(w: &World, rec: &SceneRecord) -> serde_json::Value {
    let refs = |ids: &[ObjId]| -> Vec<mush_proto::ObjRef> {
        ids.iter().map(|&id| obj_ref(w, id)).collect()
    };
    let mut value = glance(w, rec);
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "participants".into(),
            serde_json::to_value(refs(&rec.participants)).unwrap_or_default(),
        );
        object.insert(
            "observers".into(),
            serde_json::to_value(refs(&rec.observers)).unwrap_or_default(),
        );
        object.insert(
            "facilitator".into(),
            serde_json::to_value(obj_ref(w, rec.facilitator)).unwrap_or_default(),
        );
        object.insert(
            "anchor".into(),
            serde_json::to_value(obj_ref(w, rec.anchor)).unwrap_or_default(),
        );
        object.insert("created_at".into(), rec.created_at.into());
        object.insert("ic_time".into(), rec.scene_ic_time().into());
        object.insert("round_seconds".into(), rec.round_seconds.into());
        object.insert(
            "ended_at".into(),
            serde_json::to_value(rec.ended_at).unwrap_or_default(),
        );
        // D6: the consent tally as counts per subject — never who voted what
        // (that is the facilitator's, in `+timestop/who`).
        object.insert("consent".into(), rec.consent_summary());
        object.insert(
            "nudge_after_s".into(),
            serde_json::to_value(rec.nudge_after_s).unwrap_or_default(),
        );
    }
    value
}

fn panel(data: serde_json::Value) -> OutputEvent {
    OutputEvent::Panel {
        feed: SmolStr::new_static(FEED),
        kind: PanelFrameKind::Keyframe,
        data,
    }
}

/// Push the scene's keyframe to every current member (participants and
/// observers). Call after the mutation and its persist, never before.
pub(crate) fn publish(ctx: &mut Ctx<'_>, rec: &SceneRecord) {
    let data = keyframe(ctx.w, rec);
    let members: Vec<ObjId> = rec.members().collect();
    for member in members {
        ctx.out.to_actor(member, panel(data.clone()));
    }
}

/// Tell a member who just left the scene that they have none.
pub(crate) fn publish_none(ctx: &mut Ctx<'_>, who: ObjId) {
    ctx.out.to_actor(who, panel(serde_json::Value::Null));
}

/// The `scenes` query: the actor's active scene as a keyframe, or `null`
/// when they are in none.
pub(crate) struct ScenesQuery;

impl QueryHandler for ScenesQuery {
    fn query(
        &self,
        w: &World,
        actor: ObjId,
        _params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        let data = w
            .module_state::<ScenesState>()
            .and_then(|st| st.active_scene_of(actor))
            .map_or(serde_json::Value::Null, |rec| keyframe(w, rec));
        Ok(panel(data))
    }
}

/// The `RoomAmbient` hook: the glance of the active scene anchored to the
/// rendered room, if any.
pub(crate) struct SceneAmbient;

impl EventHook for SceneAmbient {
    fn room_ambient(
        &self,
        ctx: &HookReadContext<'_>,
        _looker: ObjId,
        room: ObjId,
    ) -> Vec<(SmolStr, serde_json::Value)> {
        let w = ctx.world();
        w.module_state::<ScenesState>()
            .and_then(|st| st.active_scene_at(room))
            .map(|rec| vec![(SmolStr::new_static(AMBIENT_KEY), glance(w, rec))])
            .unwrap_or_default()
    }
}
