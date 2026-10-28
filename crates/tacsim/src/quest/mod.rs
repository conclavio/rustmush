//! Conversation-discovered, persistent favors backed by real world observations.
//! Dialogue can explain a quest; only explicit player commands advance it.

mod commands;
pub mod content;
pub(crate) mod evidence;
pub(crate) mod model;
pub mod packs;
pub(crate) mod registration;
pub(crate) mod store;

use mush_core::{AttrFlags, Ctx, Flags, GOD, ObjId, ObjKind, World};

use crate::port::passenger::{economy, store as passengers};
use model::{Checkpoint, ClosureKind, Objective, Prerequisite, QuestDef, QuestRecord};

pub const DEFINITION_ATTR: &str = "quest:definition";
pub(crate) use commands::register;

pub(crate) fn definitions(w: &World, npc: ObjId) -> Vec<QuestDef> {
    let Some(object) = w.get(npc) else {
        return Vec::new();
    };
    if object.kind != ObjKind::Npc || object.flags.contains(Flags::GOING) {
        return Vec::new();
    }
    let Some(attr) = object.attrs.get(DEFINITION_ATTR) else {
        return Vec::new();
    };
    if attr.owner != GOD {
        return Vec::new();
    }
    attr.as_json()
        .and_then(|value| model::decode_definitions(value).ok())
        .unwrap_or_default()
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

/// Availability belongs to the service, including shared travellers who may
/// already have left. Accepting a lesson never creates its prerequisites.
fn available(w: &World, def: &QuestDef) -> bool {
    def.checkpoints
        .iter()
        .all(|checkpoint| match checkpoint.objective {
            Objective::Observation => detail(w, checkpoint).is_some(),
            Objective::Passenger { npc } => {
                crate::port::passenger::traveller(w, npc).is_some_and(|traveller| {
                    w.get(npc)
                        .is_some_and(|o| o.location == Some(checkpoint.room))
                        && crate::port::passenger::at_station(w, npc, &traveller.origin)
                })
            }
            _ => true,
        })
}

/// Rechecked at action commit, after resolution may have yielded to other work.
pub(crate) fn transition_allowed(
    w: &World,
    before: Option<&QuestRecord>,
    next: &QuestRecord,
) -> bool {
    if next.check().is_err() || !w.is_connected(next.actor) {
        return false;
    }
    // Closing a journal entry never depends on a departed giver, an unavailable
    // objective or payments that may have changed since acceptance. The owner
    // still checks the exact current receipt, immutable terms and index CAS.
    if next
        .closed
        .as_ref()
        .is_some_and(|closed| closed.kind == ClosureKind::Abandoned)
    {
        return before.is_some_and(|old| {
            old.is_active() && store::progress_valid(Some(old), next) && next.checks == old.checks
        });
    }
    if match &next.definition.prerequisite {
        Prerequisite::PaidPassage => !next
            .passage
            .as_ref()
            .is_some_and(|passage| paid(w, passage, next.actor, next.npc, &next.definition)),
        Prerequisite::Open | Prerequisite::Completed { .. } => {
            next.passage.is_some() || next.key.actor != Some(next.actor)
        }
    } {
        return false;
    }
    match before {
        None => {
            next.revision == 1
                && next.checks.is_empty()
                && next.completed.is_none()
                && definitions(w, next.npc).contains(&next.definition)
                && close_world(w, next.npc, next.actor)
                && crate::port::passenger::at_station(w, next.npc, &next.definition.destination)
                && available(w, &next.definition)
        }
        Some(old) => {
            if !old.is_active()
                || old.revision.checked_add(1) != Some(next.revision)
                || old.key != next.key
                || old.actor != next.actor
                || old.npc != next.npc
                || old.passage != next.passage
                || old.prerequisite_receipt != next.prerequisite_receipt
                || old.definition != next.definition
                || old.accepted != next.accepted
            {
                return false;
            }
            if next.completed.is_some() {
                return next.checks == old.checks
                    && old.checks.len() == old.definition.checkpoints.len()
                    && evidence::completion_ready(w, old)
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
            if !w.get(checkpoint.room).is_some_and(|room| {
                room.kind == ObjKind::Room && !room.flags.contains(Flags::GOING)
            }) {
                return false;
            }
            if !matches!(checkpoint.objective, Objective::Observation) {
                return w.get(next.actor).is_some_and(|p| {
                    p.location == Some(checkpoint.room) && !p.flags.contains(Flags::GOING)
                }) && !mush_sim::deployed::is_custodied(w, next.actor)
                    && evidence::guard(w, old, evidence);
            }
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
    let mut rows = store::list(ctx).unwrap_or_default();
    let actors: Vec<_> = ctx
        .w
        .connected()
        .filter(|actor| *actor != ctx.actor)
        .collect();
    for actor in actors {
        if let Ok(own) = store::refresh_for(ctx, actor) {
            rows.extend(own);
        }
    }
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    rows.dedup_by(|a, b| a.key == b.key);
    for row in rows.iter().filter(|row| row.is_active()) {
        let _ = evidence::watch(ctx, row);
    }
    evidence::reconcile(ctx);
    for row in rows.into_iter().filter(|r| r.completed.is_some()) {
        if !evidence::completion_ready(ctx.w, &row) {
            continue;
        }
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
