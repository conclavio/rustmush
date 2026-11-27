//! Live WoW-style NPC quest markers (`RoomView.ambient.quest_markers`) and the
//! structured `"tacsim.quests"` panel feed for Unity, Web, and SSH TUI clients.
//!
//! All data is computed directly from authoritative typed quest state
//! (`tacsim.quest`, `tacsim.quest_index`, `tacsim.quest_survey_evidence`,
//! `tacsim.surveys`, `tacsim.passengers`). Every action command emitted in the
//! feed (`accept_cmd`, `decline_cmd`, `check_cmd`, `finish_cmd`, `abandon_cmd`)
//! is a signed, verified `+quest/...` verb.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use mush_core::module::{
    EventHook, HookPoint, HookReadContext, ModError, PanelFeedHandle, QueryHandler, Registrar,
    WorldEvent, WorldEventKind, WorldObserver,
};
use mush_core::{Audience, CanonicalId, Ctx, Flags, ObjId, ObjKind, World};
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use serde_json::{Value, json};
use smol_str::SmolStr;

use super::evidence::{SurveyEvidenceBook, checkpoint_live_ready};
use super::model::{
    ClosureKind, Objective, Prerequisite, QuestDef, QuestIndexKey, QuestKey, QuestRecord,
};
use super::{available, close_world, commands, definitions, evidence, paid, store};
use crate::port::families::PortHandles;
use crate::port::passenger::store::{Booking, BookingKey};
use crate::surface::survey::{SurveyKey, SurveyRecord};

/// Structured panel feed carrying per-player active quests, available offers at
/// the current room, completed quest history, and live room NPC quest markers.
pub const QUESTS_FEED: &str = "tacsim.quests";

/// Ambient key merged into `RoomView.ambient` for rooms containing quest NPCs.
pub const ROOM_MARKERS_AMBIENT_KEY: &str = "quest_markers";

const MAX_FINGERPRINT_ACTORS: usize = 8_192;

#[derive(Clone)]
struct QuestPanelState {
    feed: PanelFeedHandle,
    fingerprints: BTreeMap<ObjId, u64>,
}

/// Register the `"tacsim.quests"` panel feed, query handler, `RoomAmbient`
/// marker hook, and live push observers (`Moved`, `ActionStateChanged`).
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    let feed = reg.panel_feed(QUESTS_FEED)?;
    reg.state(QuestPanelState {
        feed,
        fingerprints: BTreeMap::new(),
    });
    reg.query_handler(QUESTS_FEED, Arc::new(QuestsQuery))?;
    reg.hook(HookPoint::RoomAmbient, Arc::new(QuestAmbientHook));
    reg.subscribe(
        WorldEventKind::ActionStateChanged,
        Arc::new(QuestPushObserver),
    );
    Ok(())
}

struct QuestSnapshot {
    own_rows: Vec<QuestRecord>,
    survey_book: Option<SurveyEvidenceBook>,
    bookings: Vec<Booking>,
    commissions: Vec<SurveyRecord>,
}

fn read_snapshot_from_hook(ctx: &HookReadContext<'_>, actor: ObjId) -> QuestSnapshot {
    let w = ctx.world();
    let Some(handles) = w.module_state::<PortHandles>() else {
        return fallback_snapshot(w, actor);
    };
    let mut own_rows = Vec::new();
    let mut readable = false;
    for scope in [QuestIndexKey::Legacy, QuestIndexKey::Actor(actor)] {
        if let Ok(Some(index)) = ctx.action_state(&handles.families.quest_index, &scope) {
            readable = true;
            for key in &index.keys {
                if let Ok(Some(row)) = ctx.action_state(&handles.families.quests, key)
                    && row.actor == actor
                    && row.check().is_ok()
                {
                    own_rows.push(row.clone());
                }
            }
        } else if ctx
            .action_state(&handles.families.quest_index, &scope)
            .is_ok()
        {
            readable = true;
        }
    }
    if !readable {
        return fallback_snapshot(w, actor);
    }
    let survey_book = ctx
        .action_state(&handles.families.quest_survey_evidence, &actor)
        .ok()
        .flatten()
        .cloned();
    let mut bookings = Vec::new();
    if let Ok(Some(p_index)) = ctx.action_state(&handles.families.passenger_index, &0_u8) {
        for &id in p_index.current.values() {
            if let Ok(Some(row)) = ctx.action_state(&handles.families.passengers, &BookingKey(id))
                && row.check().is_ok()
            {
                bookings.push(row.clone());
            }
        }
    }
    let mut commissions = Vec::new();
    for offer in crate::port::survey::offers() {
        if let Some(key) = SurveyKey::parse(offer.id)
            && let Ok(Some(row)) = ctx.action_state(&handles.families.surveys, &key)
            && row.check().is_ok()
        {
            commissions.push(row.clone());
        }
    }
    QuestSnapshot {
        own_rows,
        survey_book,
        bookings,
        commissions,
    }
}

fn read_snapshot_from_ctx(ctx: &Ctx<'_>, actor: ObjId) -> QuestSnapshot {
    let w: &World = ctx.w;
    let Some(handles) = w.module_state::<PortHandles>() else {
        return fallback_snapshot(w, actor);
    };
    let own_rows = store::list_for(ctx, actor)
        .unwrap_or_else(|_| store::list_world(w))
        .into_iter()
        .filter(|row| row.actor == actor)
        .collect();
    let survey_book = ctx
        .action_state(&handles.families.quest_survey_evidence, &actor)
        .ok()
        .flatten()
        .cloned();
    let bookings = crate::port::passenger::store::list(ctx)
        .unwrap_or_else(|_| crate::port::passenger::store::list_world(w));
    let commissions = crate::port::survey::records(ctx).unwrap_or_else(|_| {
        store::commission_rows_world(w)
            .map(<[SurveyRecord]>::to_vec)
            .unwrap_or_default()
    });
    QuestSnapshot {
        own_rows,
        survey_book,
        bookings,
        commissions,
    }
}

fn fallback_snapshot(w: &World, actor: ObjId) -> QuestSnapshot {
    let own_rows = store::list_world(w)
        .into_iter()
        .filter(|row| row.actor == actor)
        .collect();
    let bookings = crate::port::passenger::store::list_world(w);
    let commissions = store::commission_rows_world(w)
        .map(<[SurveyRecord]>::to_vec)
        .unwrap_or_default();
    QuestSnapshot {
        own_rows,
        survey_book: None,
        bookings,
        commissions,
    }
}

struct EligibleOffer {
    npc: ObjId,
    definition: QuestDef,
    passage: Option<Booking>,
    prerequisite_receipt: Option<QuestKey>,
}

fn check_offer_eligible(
    w: &World,
    actor: ObjId,
    npc: ObjId,
    def: QuestDef,
    snap: &QuestSnapshot,
    at_station_fn: &impl Fn(&World, ObjId, &str) -> bool,
) -> Option<EligibleOffer> {
    if !at_station_fn(w, npc, &def.destination) || !available(w, &def) {
        return None;
    }
    for checkpoint in &def.checkpoints {
        if let Objective::PaidPassage {
            npc: traveller_npc, ..
        } = &checkpoint.objective
            && !evidence::paid_passage_available(
                w,
                actor,
                &checkpoint.objective,
                snap.bookings.iter().find(|b| b.npc == *traveller_npc),
            )
        {
            return None;
        }
    }
    if def
        .checkpoints
        .iter()
        .any(|c| c.objective == Objective::SurveyCommission)
        && !crate::port::survey::quest_available(w, actor, &snap.commissions)
    {
        return None;
    }
    let passage = if def.prerequisite == Prerequisite::PaidPassage {
        Some(
            snap.bookings
                .iter()
                .find(|b| paid(w, b, actor, npc, &def))?
                .clone(),
        )
    } else {
        None
    };
    if snap.own_rows.iter().any(|r| {
        r.key.source == def.key
            && match &passage {
                Some(booking) => r.key.booking == Some(booking.id),
                None => r.key.actor == Some(actor),
            }
            && !r.is_restartable_abandoned()
    }) {
        return None;
    }
    if let Some(choice) = &def.choice
        && snap.own_rows.iter().any(|r| {
            !r.closed
                .as_ref()
                .is_some_and(|closed| closed.kind == ClosureKind::Declined)
                && r.definition
                    .choice
                    .as_ref()
                    .is_some_and(|taken| taken.group == choice.group && taken.path != choice.path)
        })
    {
        return None;
    }
    let prerequisite_receipt = match &def.prerequisite {
        Prerequisite::Completed { quest } => Some(
            snap.own_rows
                .iter()
                .find(|row| {
                    row.key.source == *quest && row.completed.is_some() && row.closed.is_none()
                })?
                .key
                .clone(),
        ),
        _ => None,
    };
    Some(EligibleOffer {
        npc,
        definition: def,
        passage,
        prerequisite_receipt,
    })
}

fn npc_visible_in_room(w: &World, room: ObjId, npc: ObjId) -> bool {
    let Some(object) = w.get(npc) else {
        return false;
    };
    let enabled = object
        .attrs
        .get(mush_npc::PROFILE_ATTR)
        .and_then(|a| a.as_json())
        .and_then(|v| serde_json::from_value::<mush_npc::AgentProfile>(v.clone()).ok())
        .is_some_and(|p| p.enabled && p.validate().is_ok());
    enabled
        && object.kind == ObjKind::Npc
        && object.location == Some(room)
        && !object.flags.intersects(Flags::GOING | Flags::DARK)
        && !mush_sim::deployed::is_custodied(w, npc)
}

fn row_ready_at_turn_in(w: &World, row: &QuestRecord, snap: &QuestSnapshot) -> bool {
    if !row.is_active() {
        return false;
    }
    if row.checks.len() == row.definition.checkpoints.len() {
        return evidence::completion_ready(w, row);
    }
    // For guided quests, if every remaining objective is already satisfied live
    // and can be verified right at the turn-in NPC, treat the turn-in NPC as ready!
    if row.definition.is_guided() {
        let remaining = row
            .definition
            .checkpoints
            .get(row.checks.len()..)
            .unwrap_or(&[]);
        return !remaining.is_empty()
            && remaining.iter().all(|cp| {
                !matches!(
                    cp.objective,
                    Objective::Observation
                        | Objective::Passenger { .. }
                        | Objective::Contact { .. }
                ) && checkpoint_live_ready(
                    w,
                    row,
                    cp,
                    snap.survey_book.as_ref(),
                    &snap.bookings,
                    &snap.commissions,
                )
            });
    }
    false
}

fn category_label(def: &QuestDef) -> &str {
    def.category.as_deref().unwrap_or("optional")
}

fn room_markers_for(
    w: &World,
    actor: ObjId,
    room: ObjId,
    snap: &QuestSnapshot,
    can_perceive_fn: impl Fn(ObjId) -> bool,
    at_station_fn: &impl Fn(&World, ObjId, &str) -> bool,
) -> (Vec<Value>, Vec<EligibleOffer>) {
    let mut markers = Vec::new();
    let mut all_offers = Vec::new();
    let npcs: Vec<ObjId> = w
        .contents_of(room)
        .iter()
        .copied()
        .filter(|&id| npc_visible_in_room(w, room, id) && can_perceive_fn(id))
        .collect();
    for npc in npcs {
        let mut turn_in_quests: Vec<&QuestRecord> = snap
            .own_rows
            .iter()
            .filter(|r| r.is_active() && r.turn_in_npc() == npc && row_ready_at_turn_in(w, r, snap))
            .collect();
        turn_in_quests.sort_by(|a, b| {
            b.definition
                .is_guided()
                .cmp(&a.definition.is_guided())
                .then_with(|| a.definition.key.cmp(&b.definition.key))
        });

        let mut offers: Vec<EligibleOffer> = definitions(w, npc)
            .into_iter()
            .filter_map(|def| check_offer_eligible(w, actor, npc, def, snap, at_station_fn))
            .collect();
        offers.sort_by(|a, b| {
            b.definition
                .is_guided()
                .cmp(&a.definition.is_guided())
                .then_with(|| a.definition.key.cmp(&b.definition.key))
        });

        let mut in_progress: Vec<&QuestRecord> = snap
            .own_rows
            .iter()
            .filter(|r| r.is_active() && (r.npc == npc || r.turn_in_npc() == npc))
            .collect();
        in_progress.sort_by(|a, b| {
            b.definition
                .is_guided()
                .cmp(&a.definition.is_guided())
                .then_with(|| a.definition.key.cmp(&b.definition.key))
        });

        let npc_name = crate::port::coop::name_of(w, npc);
        if let Some(first) = turn_in_quests.first() {
            markers.push(json!({
                "npc_id": format!("#{}", npc.0),
                "npc_name": npc_name,
                "marker": "turn_in",
                "symbol": "?",
                "quest_key": first.definition.key,
                "quest_title": first.definition.title,
                "category": category_label(&first.definition),
                "quest_count": turn_in_quests.len(),
            }));
        } else if let Some(first) = offers.first() {
            markers.push(json!({
                "npc_id": format!("#{}", npc.0),
                "npc_name": npc_name,
                "marker": "available",
                "symbol": "!",
                "quest_key": first.definition.key,
                "quest_title": first.definition.title,
                "category": category_label(&first.definition),
                "quest_count": offers.len(),
            }));
        } else if let Some(first) = in_progress.first() {
            markers.push(json!({
                "npc_id": format!("#{}", npc.0),
                "npc_name": npc_name,
                "marker": "in_progress",
                "symbol": "?",
                "quest_key": first.definition.key,
                "quest_title": first.definition.title,
                "category": category_label(&first.definition),
                "quest_count": in_progress.len(),
            }));
        }
        all_offers.extend(offers);
    }
    (markers, all_offers)
}

fn reward_summary_for(w: &World, def: &QuestDef, turn_in: ObjId) -> String {
    if let Some(summary) = &def.reward_summary {
        return summary.clone();
    }
    let name = crate::port::coop::name_of(w, turn_in);
    format!("+10 trust and +5 disposition with {name} (once per quest)")
}

fn build_quests_payload(
    w: &World,
    actor: ObjId,
    snap: &QuestSnapshot,
    can_perceive_fn: impl Fn(ObjId) -> bool,
) -> (Value, u64) {
    build_quests_payload_with(
        w,
        actor,
        snap,
        can_perceive_fn,
        &crate::port::passenger::at_station,
    )
}

fn build_quests_payload_with(
    w: &World,
    actor: ObjId,
    snap: &QuestSnapshot,
    can_perceive_fn: impl Fn(ObjId) -> bool,
    at_station_fn: &impl Fn(&World, ObjId, &str) -> bool,
) -> (Value, u64) {
    let room = w.get(actor).and_then(|o| o.location);
    let (room_id_str, room_name_str, room_markers, offers) = match room {
        Some(room_id) => {
            let (markers, offers) =
                room_markers_for(w, actor, room_id, snap, can_perceive_fn, at_station_fn);
            (
                format!("#{}", room_id.0),
                crate::port::coop::name_of(w, room_id),
                markers,
                offers,
            )
        }
        None => (String::new(), String::new(), Vec::new(), Vec::new()),
    };

    let mut active_rows: Vec<&QuestRecord> =
        snap.own_rows.iter().filter(|r| r.is_active()).collect();
    active_rows.sort_by(|a, b| {
        b.definition
            .is_guided()
            .cmp(&a.definition.is_guided())
            .then_with(|| a.definition.key.cmp(&b.definition.key))
    });

    let mut active_json = Vec::with_capacity(active_rows.len());
    let mut fp_hasher = std::collections::hash_map::DefaultHasher::new();
    room.hash(&mut fp_hasher);
    Value::Array(room_markers.clone())
        .to_string()
        .hash(&mut fp_hasher);

    for row in active_rows {
        let giver_id = row.npc;
        let giver_name = crate::port::coop::name_of(w, giver_id);
        let turn_in_id = row.turn_in_npc();
        let turn_in_name = crate::port::coop::name_of(w, turn_in_id);
        let turn_in_here = close_world(w, turn_in_id, actor);
        let giver_here = close_world(w, giver_id, actor);
        let all_checked = row.checks.len() == row.definition.checkpoints.len();
        let ready_to_finish = all_checked && evidence::completion_ready(w, row);

        let mut checkpoints_json = Vec::with_capacity(row.definition.checkpoints.len());
        let mut next_step = String::new();
        let mut next_instructions = String::new();
        let mut next_room_entry = String::new();
        let mut next_room_name = String::new();
        let mut next_objective_ready = false;
        let mut can_check_now = false;

        for (idx, cp) in row.definition.checkpoints.iter().enumerate() {
            let recorded = row.checks.get(idx);
            let checked = recorded.is_some();
            let live_ready = if checked {
                true
            } else {
                checkpoint_live_ready(
                    w,
                    row,
                    cp,
                    snap.survey_book.as_ref(),
                    &snap.bookings,
                    &snap.commissions,
                )
            };
            let cp_room_name = crate::port::coop::name_of(w, cp.room);
            if idx == row.checks.len() {
                next_step.clone_from(&cp.title);
                next_instructions.clone_from(&cp.instructions);
                next_room_entry.clone_from(&cp.room_entry);
                next_room_name.clone_from(&cp_room_name);
                next_objective_ready = live_ready;
                let at_room = room == Some(cp.room);
                let guided_remote_ok = row.definition.is_guided()
                    && !matches!(
                        cp.objective,
                        Objective::Observation
                            | Objective::Passenger { .. }
                            | Objective::Contact { .. }
                    )
                    && (turn_in_here || giver_here);
                can_check_now = live_ready && (at_room || guided_remote_ok);
            }
            checkpoints_json.push(json!({
                "id": cp.id,
                "title": cp.title,
                "instructions": cp.instructions,
                "room_entry": cp.room_entry,
                "room_name": cp_room_name,
                "checked": checked,
                "live_ready": live_ready,
                "detail_text": recorded.map(|c| c.detail_text.as_str()),
            }));
        }

        if all_checked {
            next_step = format!("Turn in to {turn_in_name}");
            next_instructions = format!(
                "Speak with {turn_in_name} and turn in this quest (+quest/finish #{}={}).",
                turn_in_id.0, row.definition.key
            );
            if let Some(turn_in_room) = w.get(turn_in_id).and_then(|o| o.location) {
                next_room_name = crate::port::coop::name_of(w, turn_in_room);
                if let Some(entry) = room_entry_for_obj(w, turn_in_room) {
                    next_room_entry = entry;
                }
            }
        }

        let check_target = if turn_in_here { turn_in_id } else { giver_id };
        let check_cmd = format!("+quest/check #{}={}", check_target.0, row.definition.key);
        let finish_cmd = format!("+quest/finish #{}={}", turn_in_id.0, row.definition.key);
        let abandon_cmd = format!("+quest/abandon #{}={}", giver_id.0, row.definition.key);
        let can_finish_now = ready_to_finish && turn_in_here;

        row.key.source.hash(&mut fp_hasher);
        row.revision.hash(&mut fp_hasher);
        next_objective_ready.hash(&mut fp_hasher);
        can_check_now.hash(&mut fp_hasher);
        can_finish_now.hash(&mut fp_hasher);

        active_json.push(json!({
            "key": row.definition.key,
            "title": row.definition.title,
            "category": category_label(&row.definition),
            "introduction": row.definition.introduction,
            "reward_summary": reward_summary_for(w, &row.definition, turn_in_id),
            "giver_id": format!("#{}", giver_id.0),
            "giver_name": giver_name,
            "turn_in_id": format!("#{}", turn_in_id.0),
            "turn_in_name": turn_in_name,
            "turn_in_here": turn_in_here,
            "allow_restart": row.definition.allow_restart,
            "ready_to_finish": ready_to_finish,
            "next_step": next_step,
            "next_instructions": next_instructions,
            "next_room_entry": next_room_entry,
            "next_room_name": next_room_name,
            "next_objective_ready": next_objective_ready,
            "can_check_now": can_check_now,
            "can_finish_now": can_finish_now,
            "check_cmd": check_cmd,
            "finish_cmd": finish_cmd,
            "abandon_cmd": abandon_cmd,
            "checkpoints": checkpoints_json,
        }));
    }

    let mut available_json = Vec::with_capacity(offers.len());
    for offer in offers {
        let def = &offer.definition;
        let giver_id = offer.npc;
        let giver_name = crate::port::coop::name_of(w, giver_id);
        let turn_in_id = def.turn_in_npc.unwrap_or(giver_id);
        let turn_in_name = crate::port::coop::name_of(w, turn_in_id);
        let Ok(token) = commands::compute_offer_token(
            w,
            actor,
            def,
            offer.passage.as_ref(),
            offer.prerequisite_receipt.as_ref(),
        ) else {
            continue;
        };
        let checkpoints: Vec<Value> = def
            .checkpoints
            .iter()
            .map(|cp| {
                json!({
                    "id": cp.id,
                    "title": cp.title,
                    "instructions": cp.instructions,
                    "room_entry": cp.room_entry,
                    "room_name": crate::port::coop::name_of(w, cp.room),
                })
            })
            .collect();
        def.key.hash(&mut fp_hasher);
        giver_id.0.hash(&mut fp_hasher);
        available_json.push(json!({
            "key": def.key,
            "title": def.title,
            "category": category_label(def),
            "introduction": def.introduction,
            "reward_summary": reward_summary_for(w, def, turn_in_id),
            "giver_id": format!("#{}", giver_id.0),
            "giver_name": giver_name,
            "turn_in_id": format!("#{}", turn_in_id.0),
            "turn_in_name": turn_in_name,
            "allow_restart": def.allow_restart,
            "accept_cmd": format!("+quest/accept #{}={token}", giver_id.0),
            "decline_cmd": format!("+quest/decline #{}={token}", giver_id.0),
            "checkpoints": checkpoints,
        }));
    }

    let mut completed_rows: Vec<&QuestRecord> = snap
        .own_rows
        .iter()
        .filter(|r| r.completed.is_some() && r.closed.is_none())
        .collect();
    completed_rows.sort_by(|a, b| {
        b.completed
            .as_ref()
            .map(|s| s.at_utc)
            .cmp(&a.completed.as_ref().map(|s| s.at_utc))
            .then_with(|| a.definition.key.cmp(&b.definition.key))
    });
    let mut completed_json = Vec::with_capacity(completed_rows.len());
    for row in completed_rows {
        let turn_in_id = row.turn_in_npc();
        row.key.source.hash(&mut fp_hasher);
        completed_json.push(json!({
            "key": row.definition.key,
            "title": row.definition.title,
            "category": category_label(&row.definition),
            "giver_name": crate::port::coop::name_of(w, row.npc),
            "turn_in_name": crate::port::coop::name_of(w, turn_in_id),
            "completion": row.definition.completion,
            "reward_summary": reward_summary_for(w, &row.definition, turn_in_id),
            "completed_at_utc": row.completed.as_ref().map_or(0, |s| s.at_utc),
        }));
    }

    let payload = json!({
        "schema": 1,
        "room_id": room_id_str,
        "room_name": room_name_str,
        "room_markers": room_markers,
        "active": active_json,
        "available": available_json,
        "completed": completed_json,
    });
    (payload, fp_hasher.finish())
}

fn room_entry_for_obj(w: &World, room: ObjId) -> Option<String> {
    let content = w.module_state::<mush_content::ContentState>()?;
    for pack_id in [
        "neighborhood.ceres_freeport",
        "neighborhood.stickney_yards",
        "neighborhood.aitken_highport",
        "neighborhood.voltaire_anchorage",
        "neighborhood.isidis_concession",
        "neighborhood.ligeia_terminal",
    ] {
        let Ok(id) = CanonicalId::new(pack_id) else {
            continue;
        };
        let Some(latch) = content.latches.applied(&id) else {
            continue;
        };
        for (entry_id, applied) in latch.entries() {
            if applied.kind().as_str() == "room" && applied.object() == Some(room) {
                return Some(entry_id.as_str().to_owned());
            }
        }
    }
    None
}

struct QuestsQuery;

impl QueryHandler for QuestsQuery {
    fn query(&self, w: &World, actor: ObjId, _params: &Value) -> Result<OutputEvent, RichText> {
        let snap = fallback_snapshot(w, actor);
        let (data, _) = build_quests_payload(w, actor, &snap, |_| true);
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(QUESTS_FEED),
            kind: PanelFrameKind::Keyframe,
            data,
        })
    }

    fn query_with_context(
        &self,
        context: &HookReadContext<'_>,
        actor: ObjId,
        _params: &Value,
    ) -> Result<OutputEvent, RichText> {
        let snap = read_snapshot_from_hook(context, actor);
        let (data, _) = build_quests_payload(context.world(), actor, &snap, |npc| {
            context.can_perceive(actor, npc)
        });
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(QUESTS_FEED),
            kind: PanelFrameKind::Keyframe,
            data,
        })
    }
}

struct QuestAmbientHook;

impl EventHook for QuestAmbientHook {
    fn room_ambient(
        &self,
        ctx: &HookReadContext<'_>,
        looker: ObjId,
        room: ObjId,
    ) -> Vec<(SmolStr, Value)> {
        let w = ctx.world();
        if !w
            .get(looker)
            .is_some_and(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
        {
            return Vec::new();
        }
        let snap = read_snapshot_from_hook(ctx, looker);
        let (markers, _) = room_markers_for(
            w,
            looker,
            room,
            &snap,
            |npc| ctx.can_perceive(looker, npc),
            &crate::port::passenger::at_station,
        );
        if markers.is_empty() {
            return Vec::new();
        }
        vec![(
            SmolStr::new_static(ROOM_MARKERS_AMBIENT_KEY),
            Value::Array(markers),
        )]
    }
}

fn push_actor(ctx: &mut Ctx<'_>, actor: ObjId, force: bool) {
    if !ctx.w.is_connected(actor) {
        return;
    }
    if !ctx
        .w
        .get(actor)
        .is_some_and(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
    {
        return;
    }
    let snap = read_snapshot_from_ctx(ctx, actor);
    let (data, fp) = build_quests_payload(ctx.w, actor, &snap, |npc| ctx.can_perceive(actor, npc));
    let Some(state) = ctx.w.module_state_mut::<QuestPanelState>() else {
        return;
    };
    if !force && state.fingerprints.get(&actor) == Some(&fp) {
        return;
    }
    if state.fingerprints.len() >= MAX_FINGERPRINT_ACTORS
        && !state.fingerprints.contains_key(&actor)
    {
        state.fingerprints.clear();
    }
    state.fingerprints.insert(actor, fp);
    let feed = state.feed.clone();
    feed.keyframe_to(ctx, Audience::Actor(actor), data);
}

pub(crate) fn push_connected_if_changed(ctx: &mut Ctx<'_>) {
    let connected: Vec<ObjId> = ctx.w.connected().collect();
    let connected_set: BTreeSet<ObjId> = connected.iter().copied().collect();
    if let Some(state) = ctx.w.module_state_mut::<QuestPanelState>() {
        state
            .fingerprints
            .retain(|id, _| connected_set.contains(id));
    }
    for actor in connected {
        push_actor(ctx, actor, false);
    }
}

struct QuestPushObserver;

impl WorldObserver for QuestPushObserver {
    fn on_event(&self, ctx: &mut Ctx<'_>, ev: &WorldEvent) {
        match ev {
            WorldEvent::Moved { who, .. } if ctx.w.is_connected(*who) => {
                push_actor(ctx, *who, true);
            }
            WorldEvent::ActionStateChanged { owner, family, .. }
                if owner.as_str() == crate::NS
                    && matches!(
                        family.as_str(),
                        "tacsim.quest"
                            | "tacsim.quest_index"
                            | "tacsim.quest_survey_evidence"
                            | "tacsim.surveys"
                            | "tacsim.passengers"
                    ) =>
            {
                push_connected_if_changed(ctx);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quest::evidence::{MineReceipt, ObjectiveProof, SurveyEvidenceBook, TradeReceipt};
    use crate::quest::model::{
        CheckEvidence, Checkpoint, ClosedQuest, ClosureKind, Objective, Prerequisite, QuestDef,
        QuestKey, QuestRecord, Stamp, detail_hash,
    };
    use mush_core::{Attr, GOD};

    fn make_guided_def(npc_turn_in: Option<ObjId>, room: ObjId) -> QuestDef {
        QuestDef {
            schema: 1,
            key: "tacsim.quests.guided_onboarding/quest.test_step".into(),
            title: "Test Guided Step".into(),
            introduction: "Walk through this optional guided step.".into(),
            destination: "ceres".into(),
            prerequisite: Prerequisite::Open,
            choice: None,
            category: Some("guided".into()),
            reward_summary: Some("Guided Step Reward".into()),
            turn_in_npc: npc_turn_in,
            turn_in_destination: npc_turn_in.map(|_| "ceres".into()),
            allow_restart: true,
            checkpoints: vec![Checkpoint {
                id: "buy_fuel".into(),
                title: "Buy propellant".into(),
                room_pack: "neighborhood.ceres_freeport".into(),
                room_entry: "room.c07_exchange".into(),
                room,
                detail: String::new(),
                objective: Objective::MarketPurchase {
                    market: "ceres".into(),
                    commodity: "propellant".into(),
                    quantity: 5,
                },
                instructions: "Buy 5 tonnes of propellant.".into(),
            }],
            completion: "Step completed cleanly.".into(),
        }
    }

    #[test]
    fn new_objective_proofs_validate_purchase_sale_refit_and_surface_mine() {
        let accepted = Stamp {
            at_ic: 100,
            at_utc: 100,
        };
        let checked = Stamp {
            at_ic: 120,
            at_utc: 120,
        };
        let buy_obj = Objective::MarketPurchase {
            market: "ceres".into(),
            commodity: "provisions".into(),
            quantity: 10,
        };
        let buy_proof = ObjectiveProof::MarketPurchase {
            purchase: TradeReceipt {
                actor: ObjId(7),
                quote: 1,
                contract: 1,
                transaction: 1,
                market: "ceres".into(),
                commodity: "provisions".into(),
                quantity: 10,
                hold: 1,
                purchase: true,
                accepted_at: 105,
                settled_at: 110,
            },
        };
        assert!(
            buy_proof
                .check_for(&buy_obj, ObjId(7), &accepted, &checked)
                .is_ok()
        );
        assert!(
            buy_proof
                .check_for(&buy_obj, ObjId(8), &accepted, &checked)
                .is_err()
        );

        let sell_obj = Objective::MarketSale {
            market: "stickney".into(),
            commodity: "carbon_ore".into(),
            quantity: 1,
        };
        let sell_proof = ObjectiveProof::MarketSale {
            sale: TradeReceipt {
                actor: ObjId(7),
                quote: 2,
                contract: 2,
                transaction: 2,
                market: "stickney".into(),
                commodity: "carbon_ore".into(),
                quantity: 2,
                hold: 1,
                purchase: false,
                accepted_at: 105,
                settled_at: 115,
            },
        };
        assert!(
            sell_proof
                .check_for(&sell_obj, ObjId(7), &accepted, &checked)
                .is_ok()
        );

        let refit_obj = Objective::ShipRefit {
            unit: "tacsim.unit.dray_p".into(),
        };
        let refit_proof = ObjectiveProof::ShipRefit {
            actor: ObjId(7),
            hull: ObjId(20),
            unit: "tacsim.unit.dray_p".into(),
        };
        assert!(
            refit_proof
                .check_for(&refit_obj, ObjId(7), &accepted, &checked)
                .is_ok()
        );

        let mine_obj = Objective::SurfaceMine {
            commodity: "carbon_ore".into(),
            quantity: 1,
        };
        let mine_receipt = MineReceipt {
            actor: ObjId(7),
            tv: ObjId(30),
            ordinal: 3,
            site: CanonicalId::new("ahuna_port").unwrap(),
            commodity: "carbon_ore".into(),
            quantity: 2,
            due_ic: 110,
            at_utc: 112,
        };
        let mine_proof = ObjectiveProof::SurfaceMine {
            receipt: mine_receipt.clone(),
        };
        assert!(
            mine_proof
                .check_for(&mine_obj, ObjId(7), &accepted, &checked)
                .is_ok()
        );
        let book = SurveyEvidenceBook {
            schema: 1,
            actor: ObjId(7),
            scan: None,
            sound: None,
            mined: vec![mine_receipt],
        };
        assert!(book.check().is_ok());
        assert!(evidence::proof_current(Some(&book), ObjId(7), &mine_proof));
    }

    #[test]
    fn guided_quests_can_abandon_and_restart_while_non_guided_stay_terminal() {
        let def = make_guided_def(None, ObjId(20));
        let mut active = QuestRecord {
            schema: 1,
            key: QuestKey {
                source: def.key.clone(),
                booking: None,
                actor: Some(ObjId(11)),
            },
            revision: 1,
            actor: ObjId(11),
            npc: ObjId(10),
            passage: None,
            prerequisite_receipt: None,
            definition: def,
            accepted: Stamp {
                at_ic: 10,
                at_utc: 10,
            },
            checks: Vec::new(),
            completed: None,
            closed: None,
            restart_count: 0,
        };
        assert!(store::progress_valid(None, &active));

        let mut abandoned = active.clone();
        abandoned.revision = 2;
        abandoned.closed = Some(ClosedQuest {
            kind: ClosureKind::Abandoned,
            at: Stamp {
                at_ic: 20,
                at_utc: 20,
            },
        });
        assert!(store::progress_valid(Some(&active), &abandoned));
        assert!(abandoned.is_restartable_abandoned());

        let mut restarted = abandoned.clone();
        restarted.revision = 3;
        restarted.restart_count = 1;
        restarted.accepted = Stamp {
            at_ic: 30,
            at_utc: 30,
        };
        restarted.closed = None;
        assert!(store::progress_valid(Some(&abandoned), &restarted));

        // Legacy non-restartable quests cannot reopen once abandoned.
        active.definition.allow_restart = false;
        abandoned.definition.allow_restart = false;
        restarted.definition.allow_restart = false;
        assert!(!abandoned.is_restartable_abandoned());
        assert!(!store::progress_valid(Some(&abandoned), &restarted));
    }

    #[test]
    fn quest_markers_prioritize_turn_in_over_available_over_in_progress() {
        let mut w = World::new_minimal();
        let room = w
            .create(ObjKind::Room, "Port Watch Arrivals", GOD, None)
            .unwrap();
        let player = w
            .create(ObjKind::Player, "Captain", GOD, Some(room))
            .unwrap();
        let npc = w
            .create(ObjKind::Npc, "Maren Halvorsen", GOD, Some(room))
            .unwrap();
        w.set_connected(player, true);
        w.set_attr(
            npc,
            mush_npc::PROFILE_ATTR,
            Attr::json(json!(mush_npc::AgentProfile::default()), GOD),
        )
        .unwrap();

        let def = make_guided_def(Some(npc), room);
        w.set_attr(
            npc,
            crate::quest::DEFINITION_ATTR,
            Attr::json(serde_json::to_value(&def).unwrap(), GOD),
        )
        .unwrap();

        // 1. No active quest: marker is "available" ("!") with signed accept_cmd!
        let empty_snap = QuestSnapshot {
            own_rows: Vec::new(),
            survey_book: None,
            bookings: Vec::new(),
            commissions: Vec::new(),
        };
        let (payload, _) =
            build_quests_payload_with(&w, player, &empty_snap, |_| true, &|_, _, _| true);
        assert_eq!(payload["room_markers"][0]["marker"], "available");
        assert_eq!(payload["room_markers"][0]["symbol"], "!");
        assert!(
            payload["available"][0]["accept_cmd"]
                .as_str()
                .unwrap()
                .starts_with(&format!("+quest/accept #{}=", npc.0))
        );

        // 2. Active quest not yet checked: marker is "in_progress" ("?").
        let active_row = QuestRecord {
            schema: 1,
            key: QuestKey {
                source: def.key.clone(),
                booking: None,
                actor: Some(player),
            },
            revision: 1,
            actor: player,
            npc,
            passage: None,
            prerequisite_receipt: None,
            definition: def.clone(),
            accepted: Stamp {
                at_ic: 10,
                at_utc: 10,
            },
            checks: Vec::new(),
            completed: None,
            closed: None,
            restart_count: 0,
        };
        let in_prog_snap = QuestSnapshot {
            own_rows: vec![active_row.clone()],
            survey_book: None,
            bookings: Vec::new(),
            commissions: Vec::new(),
        };
        let (payload, _) =
            build_quests_payload_with(&w, player, &in_prog_snap, |_| true, &|_, _, _| true);
        assert_eq!(payload["room_markers"][0]["marker"], "in_progress");
        assert_eq!(payload["room_markers"][0]["symbol"], "?");
        assert_eq!(payload["active"].as_array().unwrap().len(), 1);

        // 3. All checkpoints verified: marker upgrades to "turn_in" ("?")!
        let mut ready_row = active_row;
        ready_row.revision = 2;
        ready_row.checks.push(CheckEvidence {
            checkpoint: "buy_fuel".into(),
            room,
            detail_hash: detail_hash("Verified.").unwrap(),
            detail_text: "Verified.".into(),
            at: Stamp {
                at_ic: 20,
                at_utc: 20,
            },
            proof: None,
        });
        let turn_in_snap = QuestSnapshot {
            own_rows: vec![ready_row],
            survey_book: None,
            bookings: Vec::new(),
            commissions: Vec::new(),
        };
        let (payload, _) =
            build_quests_payload_with(&w, player, &turn_in_snap, |_| true, &|_, _, _| true);
        assert_eq!(payload["room_markers"][0]["marker"], "turn_in");
        assert_eq!(payload["room_markers"][0]["symbol"], "?");
        assert_eq!(payload["active"][0]["can_finish_now"], true);
    }
}
