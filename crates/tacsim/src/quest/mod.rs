//! Conversation-discovered, persistent favors backed by real world observations.
//! Dialogue can explain a quest; only explicit player commands advance it.

mod commands;
pub mod content;
pub(crate) mod model;
pub(crate) mod registration;
pub(crate) mod store;

use mush_core::{AttrFlags, Ctx, Flags, GOD, ObjId, ObjKind, World};

use crate::port::passenger::{economy, store as passengers};
use model::{Checkpoint, QuestDef, QuestRecord};

pub const DEFINITION_ATTR: &str = "quest:definition";
pub(crate) use commands::register;

pub(crate) fn definition(w: &World, npc: ObjId) -> Option<QuestDef> {
    let object = w.get(npc)?;
    if object.kind != ObjKind::Npc || object.flags.contains(Flags::GOING) {
        return None;
    }
    let attr = object.attrs.get(DEFINITION_ATTR)?;
    if attr.owner != GOD {
        return None;
    }
    let def: QuestDef = serde_json::from_value(attr.as_json()?.clone()).ok()?;
    def.check().ok()?;
    Some(def)
}

fn close_world(w: &World, npc: ObjId, actor: ObjId) -> bool {
    let Some(object) = w.get(npc) else {
        return false;
    };
    let enabled = object
        .attrs
        .get(mush_npc::PROFILE_ATTR)
        .and_then(|a| a.as_json())
        .and_then(|v| serde_json::from_value::<mush_npc::AgentProfile>(v.clone()).ok())
        .is_some_and(|p| p.enabled && p.validate().is_ok());
    // Sim custody hides a deployed unit while retaining its old room location.
    // Recheck it here as well as at command-time perception, so yielding after
    // resolution cannot turn a remote tactical body into a present acquaintance.
    enabled
        && !mush_sim::deployed::is_custodied(w, npc)
        && !mush_sim::deployed::is_custodied(w, actor)
        && w.is_connected(actor)
        && object.location.is_some()
        && !object.flags.intersects(Flags::GOING | Flags::DARK)
        && w.get(actor).is_some_and(|p| {
            p.location == object.location && !p.flags.intersects(Flags::GOING | Flags::DARK)
        })
}

fn close(ctx: &Ctx<'_>, npc: ObjId) -> bool {
    close_world(ctx.w, npc, ctx.actor)
        && ctx.can_perceive(ctx.actor, npc)
        && ctx.can_perceive(npc, ctx.actor)
}

/// Read the actual public detail on the bound room, never a player-supplied claim.
pub(crate) fn detail(w: &World, checkpoint: &Checkpoint) -> Option<String> {
    let room = w.get(checkpoint.room)?;
    if room.kind != ObjKind::Room || room.flags.contains(Flags::GOING) {
        return None;
    }
    let key = format!(
        "atlas:detail:{}",
        mush_core::fold_detail_keyword(&checkpoint.detail)
    );
    let attr = room.attrs.get(&key)?;
    if attr.owner != GOD
        || attr
            .flags
            .intersects(AttrFlags::HIDDEN | AttrFlags::WIZARD_ONLY | AttrFlags::SELF_ONLY)
    {
        return None;
    }
    let text = attr.as_text()?;
    if text.trim().is_empty() || text.len() > 2048 {
        return None;
    }
    Some(text.to_owned())
}

fn paid(
    w: &World,
    booking: &passengers::Booking,
    actor: ObjId,
    npc: ObjId,
    def: &QuestDef,
) -> bool {
    booking.captain == actor
        && booking.npc == npc
        && booking.phase == passengers::Phase::Delivered
        && booking.destination == def.destination
        && economy::settled(w, booking)
}

/// Rechecked at action commit, after resolution may have yielded to other work.
pub(crate) fn transition_allowed(
    w: &World,
    before: Option<&QuestRecord>,
    next: &QuestRecord,
) -> bool {
    if next.check().is_err()
        || !w.is_connected(next.actor)
        || !paid(w, &next.passage, next.actor, next.npc, &next.definition)
    {
        return false;
    }
    match before {
        None => {
            next.revision == 1
                && next.checks.is_empty()
                && next.completed.is_none()
                && definition(w, next.npc).as_ref() == Some(&next.definition)
                && close_world(w, next.npc, next.actor)
                && crate::port::passenger::at_station(w, next.npc, &next.definition.destination)
                && next
                    .definition
                    .checkpoints
                    .iter()
                    .all(|c| detail(w, c).is_some())
        }
        Some(old) => {
            if old.completed.is_some()
                || old.revision.checked_add(1) != Some(next.revision)
                || old.key != next.key
                || old.actor != next.actor
                || old.npc != next.npc
                || old.passage != next.passage
                || old.definition != next.definition
                || old.accepted != next.accepted
            {
                return false;
            }
            if next.completed.is_some() {
                return next.checks == old.checks
                    && old.checks.len() == old.definition.checkpoints.len()
                    && close_world(w, next.npc, next.actor)
                    && crate::port::passenger::at_station(
                        w,
                        next.npc,
                        &next.definition.destination,
                    );
            }
            if next.checks.len() != old.checks.len() + 1 || !next.checks.starts_with(&old.checks) {
                return false;
            }
            let Some(checkpoint) = old.definition.checkpoints.get(old.checks.len()) else {
                return false;
            };
            let Some(evidence) = next.checks.last() else {
                return false;
            };
            w.get(next.actor).is_some_and(|p| {
                p.location == Some(checkpoint.room) && !p.flags.contains(Flags::GOING)
            }) && !mush_sim::deployed::is_custodied(w, next.actor)
                && evidence.room == checkpoint.room
                && evidence.checkpoint == checkpoint.id
                && detail(w, checkpoint).as_deref() == Some(evidence.detail_text.as_str())
                && model::detail_hash(&evidence.detail_text).ok().as_deref()
                    == Some(evidence.detail_hash.as_str())
        }
    }
}

/// A durable completed record is the receipt. The NPC owner deduplicates its
/// outcome and relationship update even after memory eviction or a restart.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    if store::refresh(ctx).is_err() {
        return;
    }
    let Ok(rows) = store::list(ctx) else { return };
    for row in rows.into_iter().filter(|r| r.completed.is_some()) {
        let Ok(hash) = mush_core::CanonicalHash::for_canonical_value(
            "tacsim.quest_reward.v1",
            1,
            &serde_json::json!(&row.key),
            1024,
        ) else {
            continue;
        };
        let cause = format!("quest:{}", hash.to_hex());
        if let Ok(true) = mush_npc::record_completed_work_experience(
            ctx,
            row.npc,
            row.actor,
            &cause,
            &row.definition.completion,
        ) {
            mush_npc::notify_role_event(
                ctx,
                row.npc,
                "A completed favor has been recorded in your own experience. Use quest.status for this asker's verified progress; do not reveal another person's private journal.",
            );
        }
    }
}
