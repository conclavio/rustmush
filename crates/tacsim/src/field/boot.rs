//! **A field across a restart** (doc 32 cut CF.2; Q-CF-33, Q-CF-34, Q-CF-35).
//!
//! The boot reads every field's live record and snapshot (`on_world_loaded`, [`read`]) and takes
//! every field-marked unit out of the match resume's way (`crate::match_::resume::phase_a` hands
//! them here, [`adopt_mark`]) — the OOC rule restores a check-out snapshot, which an IC unit
//! never takes. At `on_all_loaded` ([`phase`]) each field is decided: while the W-clock is
//! frozen (an outage's countdown) it is **held**, and resumes when the world's clock resumes
//! ([`on_resumed`], the `WorldClockResumed` observer; the tick retries anything still held), so
//! nothing is shot unseen during the countdown (FO-11). A field whose live record and snapshot
//! decode resumes from the snapshot on a fresh task — its seated pilots rebound to their seats,
//! the quiet interval restarted, the black box on a new segment's log; anything else is the IC
//! void rule: its units demote from the last snapshot, or as they went out with none.
//!
//! **The host's owed writes** (doc 32 §15): a site's closing tombstone owes its `close` again and
//! holds the site closing, so the tick never re-admits a field that ended before its close
//! landed; a live record's `owed_exits` owe their `exit`s again; a live record's pending fire
//! order is the resumed field's opening shot.

use std::collections::BTreeSet;

use mush_core::{CanonicalId, Ctx, ObjId, WorldEvent};

use super::FieldBook;
use super::record::{
    FIELD_CLOSING_FORMAT, FieldClosing, FieldLive, closing_site_of_key, live_site_of_key,
    snapshot_site_of_key,
};

/// **What the boot read for one site.**
#[derive(Clone, Debug, Default)]
pub(crate) struct BootDocs {
    /// The live record, decoded.
    pub live: Option<FieldLive>,
    /// The snapshot document.
    pub snapshot: Option<serde_json::Value>,
}

/// **A resume held while the world is frozen** (Q-CF-33).
#[derive(Clone, Debug, Default)]
pub(crate) struct Held {
    /// The documents.
    pub docs: BootDocs,
    /// The units still under the field's mark.
    pub units: BTreeSet<ObjId>,
}

/// **Read the boot's documents** into the book (`on_world_loaded`).
pub(crate) fn read(ctx: &mut Ctx<'_>, kv: &[(smol_str::SmolStr, serde_json::Value)]) {
    // Doc 32 §15 (the review's CF-1): the closes and exits the host owed when it went down, owed
    // again before anything else reads the book.
    for (key, value) in kv {
        if let Some(site) = closing_site_of_key(key.as_str()) {
            match serde_json::from_value::<FieldClosing>(value.clone()) {
                Ok(closing) if closing.site == site && closing.format == FIELD_CLOSING_FORMAT => {
                    super::admit::restore_closing(ctx.w, closing);
                }
                Ok(_) | Err(_) => {
                    tracing::warn!(site = %site, "tacsim: a field's closing tombstone did not decode; its close is lost");
                }
            }
        } else if let Some(site) = live_site_of_key(key.as_str())
            && let Ok(live) = FieldLive::from_document(value)
            && live.field.site == site
            && let Some(owed) = ctx.w.module_state_mut::<super::exit::OwedExits>()
        {
            for exit in &live.owed_exits {
                owed.0
                    .entry(site.clone())
                    .or_default()
                    .insert(exit.unit, exit.actor);
            }
        }
    }
    let Some(book) = ctx.w.module_state_mut::<FieldBook>() else {
        return;
    };
    for (key, value) in kv {
        if let Some(site) = live_site_of_key(key.as_str()) {
            match FieldLive::from_document(value) {
                Ok(live) if live.field.site == site => {
                    book.boot_docs.entry(site).or_default().live = Some(live);
                }
                Ok(_) | Err(_) => {
                    tracing::warn!(site = %site, "tacsim: a field's live record did not decode; the field will void");
                    book.boot_docs.entry(site).or_default();
                }
            }
        } else if let Some(site) = snapshot_site_of_key(key.as_str()) {
            book.boot_docs.entry(site).or_default().snapshot = Some(value.clone());
        }
    }
}

/// **A field-marked unit the boot found** (`phase_a` hands it here).
pub(crate) fn adopt_mark(w: &mut mush_core::World, site: &str, unit: ObjId) {
    let Ok(site) = CanonicalId::new(site) else {
        return;
    };
    if let Some(book) = w.module_state_mut::<FieldBook>() {
        book.boot_marks.entry(site).or_default().insert(unit);
    }
}

/// **Decide every field the boot found** (`on_all_loaded`).
pub(crate) fn phase(ctx: &mut Ctx<'_>) {
    let Some(book) = ctx.w.module_state_mut::<FieldBook>() else {
        return;
    };
    let docs = std::mem::take(&mut book.boot_docs);
    let marks = std::mem::take(&mut book.boot_marks);
    let sites: BTreeSet<CanonicalId> = docs.keys().chain(marks.keys()).cloned().collect();
    for site in sites {
        let held = Held {
            docs: docs.get(&site).cloned().unwrap_or_default(),
            units: marks.get(&site).cloned().unwrap_or_default(),
        };
        book.held.insert(site, held);
    }
    run_held(ctx);
}

/// **The world's clock resumed** (the observer): run every held resume.
pub(crate) fn on_resumed(ctx: &mut Ctx<'_>, _event: &WorldEvent) {
    run_held(ctx);
}

/// **Run every held resume** unless the world is still frozen.
pub(crate) fn run_held(ctx: &mut Ctx<'_>) {
    if ctx.w.world_timeline().is_frozen() {
        return;
    }
    let held = ctx
        .w
        .module_state_mut::<FieldBook>()
        .map(|book| std::mem::take(&mut book.held))
        .unwrap_or_default();
    for (site, held) in held {
        if let Err(why) = resume(ctx, &site, &held) {
            tracing::warn!(site = %site, reason = %why, "tacsim: a field did not resume; it voids from its last snapshot");
            void(ctx, &site, &held);
        }
    }
}

/// **Resume one field from its snapshot** (Q-CF-33).
fn resume(ctx: &mut Ctx<'_>, site: &CanonicalId, held: &Held) -> Result<(), String> {
    let mut live = held
        .docs
        .live
        .clone()
        .ok_or_else(|| "it has no live record".to_owned())?;
    let document = held
        .docs
        .snapshot
        .clone()
        .ok_or_else(|| "it has no snapshot".to_owned())?;
    let record =
        super::close::site_record(ctx, site).ok_or_else(|| "its site record is gone".to_owned())?;
    if record.open_field() != Some(live.field.clone()) {
        return Err("its site record no longer holds it open".to_owned());
    }
    let units: BTreeSet<ObjId> = live.units.iter().map(|row| row.unit).collect();
    if units != held.units {
        return Err("its units' custody marks disagree with its live record".to_owned());
    }
    let ground = super::ground::definition(ctx.w, site, &record, live.homes.clone())?;
    let limit = ctx.w.module_state::<crate::store::TacsimStore>().map_or(
        crate::arena::snapshot_capacity::SNAPSHOT_MAX_BYTES,
        |store| store.config().arena.snapshot_max_bytes,
    );
    let head = crate::arena::Snapshot::head(&document).map_err(|error| error.to_string())?;
    let mut arena = crate::arena::Snapshot::decode_with_limit(&document, ground, limit)
        .map_err(|error| error.to_string())?;
    // Doc 32 cut CF.5 (Q-CF-18): a unit that left at the ramp after this snapshot was taken is
    // checked in already and out of the live record; the arena drops it.
    let gone: Vec<ObjId> = arena
        .units
        .iter()
        .map(|state| state.id)
        .filter(|id| !units.contains(id))
        .collect();
    for unit in gone {
        arena.remove_unit(unit);
    }
    let task = ctx
        .w
        .module_state_mut::<FieldBook>()
        .map(FieldBook::next_task)
        .ok_or_else(|| "the field host is unavailable".to_owned())?;
    live.segment = live.segment.saturating_add(1);
    live.at_ms = head.at_ms;
    if let Some(book) = ctx.w.module_state_mut::<FieldBook>() {
        book.live.insert(
            site.clone(),
            super::Hosted {
                task,
                live: live.clone(),
            },
        );
        book.snapshots.insert(site.clone(), document);
        // Doc 32 §15 (the review's CF-6): a fire-promoted field resumes with its opening shot
        // still owed when the snapshot it rewinds to was taken before the shot left.
        if let Some(fire) = live.fire.clone() {
            book.fire.insert(site.clone(), fire);
        }
    }
    let terms = crate::arena_task::FieldTerms {
        disengage_ms: super::config(ctx.w).disengage_ms(),
    };
    if let Err(error) = crate::arena_bridge::resume_field_task(ctx, task, arena, terms) {
        if let Some(book) = ctx.w.module_state_mut::<FieldBook>() {
            book.live.remove(site);
        }
        return Err(format!("its task did not reopen ({error})"));
    }
    super::record::write_live(ctx, &live);
    // The seats: a pilot still sitting in the unit is its pilot again. Focus never survives a
    // boot (mush-sim's D-E7-34), so the pilot is told how to take the cockpit back.
    for row in &live.units {
        let Some((pilot, station)) = &row.pilot else {
            continue;
        };
        if super::parties::seated(ctx.w, row.unit).is_some_and(|(sat, _)| sat == *pilot) {
            if let Some(registry) = ctx
                .w
                .module_state_mut::<crate::arena_bridge::ArenaRegistry>()
            {
                registry.bind_pilot(*pilot, task, row.unit);
            }
            // A hired NPC's hull stays the doctrine's (doc 32 cut CF.6): bound for its frame,
            // never handed the seat's control.
            if super::orders::is_npc(ctx.w, *pilot) {
                continue;
            }
            crate::arena_bridge::reseat(ctx, row.unit, *pilot);
            ctx.out.sys_to_actor(
                *pilot,
                "tacsim.field",
                format!(
                    "The server restarted. Field {} on {} resumed from its last save, {} s in.                      +focus #{}/{station} takes your cockpit back.",
                    live.field.ordinal,
                    super::admit::site_name(ctx, site),
                    head.at_ms / 1_000,
                    row.unit.0
                ),
            );
        }
    }
    tracing::info!(site = %site, ordinal = live.field.ordinal, at_ms = head.at_ms, task = task.0, "tacsim: an IC field resumed from its snapshot");
    Ok(())
}

/// **The IC void rule at boot** (Q-CF-35): demote from the last snapshot, never the check-out.
fn void(ctx: &mut Ctx<'_>, site: &CanonicalId, held: &Held) {
    let units: Vec<ObjId> = held.units.iter().copied().collect();
    let homes = held
        .docs
        .live
        .as_ref()
        .map(|live| live.homes.clone())
        .unwrap_or_default();
    let returns = super::close::snapshot_returns(ctx, site, held.docs.snapshot.as_ref(), homes)
        .unwrap_or_default();
    let left = super::close::demote_units(ctx, site, &units, &returns);
    // Doc 32 cut CF.5 (Q-CF-38): the crew of every unit the live record seated.
    if let Some(live) = &held.docs.live {
        for row in &live.units {
            super::crew::check_in(
                ctx,
                site,
                row.unit,
                row.pilot.as_ref().map(|(pilot, _)| *pilot),
            );
            super::crew::check_in_aboard(ctx, site, row.unit, &row.aboard);
        }
    }
    super::record::delete(ctx, site);
    if let Some(record) = super::close::site_record(ctx, site)
        && record
            .promoted
            .as_ref()
            .is_some_and(|open| open.by.hosted())
    {
        super::admit::owe_close(ctx, site, &record, Some(left), Vec::new());
    }
}
