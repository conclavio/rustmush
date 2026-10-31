//! **Demotion and the IC void rule** (doc 32 cut CF.2; Q-CF-20, Q-CF-24, Q-CF-25, Q-CF-31,
//! Q-CF-35, Q-CF-36).
//!
//! The bridge hands a field's `Ended` here instead of to the match wrapper. Whatever ended it,
//! every unit goes home through the **IC condition plan** (doc 27a H1-A,
//! `crate::field_ops::apply_ic_combat_return`): only the custody vocabulary, components and
//! sections conditioned by their anatomy, the blanket wreck stamp stripped, the revision fenced —
//! never the OOC diff, never the check-out snapshot (Q-CF-35). Then it stands at its field pose
//! on the surface (Q-CF-31), and the record's unit index becomes every unit the field leaves on
//! the ground, at the pose it fell: an immobile, unarmed or disabled hull a **casualty**, a
//! wreck or a total loss a **wreck** (Q-CF-25; the irrecoverable hull's remains object waits for
//! the epoch close-destroyed rule, 27a H3).
//!
//! **A void** (a panic, a stall, a ledger overflow, a refused resume) demotes from **the last
//! snapshot the world holds**: its units' cells and poses are the return. With no snapshot every
//! unit checks in as it went out, which is what the snapshot would have said.

use mush_core::{CanonicalId, Ctx, ObjId};

use super::FieldBook;
use crate::arena::damage::UnitReturn;
use crate::arena::state::{EndReason, MatchEnd};
use crate::arena_task::MatchId;
use crate::field_ops::OperationalState;
use crate::surface::site_record::{LeftCondition, LeftUnit, SitePose, SiteRecord};

/// **The units the last snapshot holds**, as returns (the void rule), when it decodes.
pub(crate) fn snapshot_returns(
    ctx: &Ctx<'_>,
    site: &CanonicalId,
    document: Option<&serde_json::Value>,
) -> Option<Vec<UnitReturn>> {
    let document = document?;
    let record = site_record(ctx, site)?;
    let ground = super::ground::definition(ctx.w, site, &record).ok()?;
    let limit = ctx.w.module_state::<crate::store::TacsimStore>().map_or(
        crate::arena::snapshot_capacity::SNAPSHOT_MAX_BYTES,
        |store| store.config().arena.snapshot_max_bytes,
    );
    crate::arena::Snapshot::decode_with_limit(document, ground, limit)
        .ok()
        .map(|arena| arena.returned_units())
}

/// The site's record as the family holds it.
pub(crate) fn site_record(ctx: &Ctx<'_>, site: &CanonicalId) -> Option<SiteRecord> {
    let handles = ctx.w.module_state::<crate::port::families::PortHandles>()?;
    let key = crate::surface::site_record::SiteKey(site.clone());
    ctx.action_state(&handles.families.sites, &key)
        .ok()
        .flatten()
        .cloned()
        .filter(|record| record.key() == key)
}

/// **A field's `Ended`** (the bridge's branch): check every unit in, untrack the task, owe the
/// record its `close`, tell the ground.
pub(crate) fn on_ended(ctx: &mut Ctx<'_>, task: MatchId, end: MatchEnd, units: &[UnitReturn]) {
    let Some(site) = ctx
        .w
        .module_state::<FieldBook>()
        .and_then(|book| book.site_of_task(task).cloned())
    else {
        return;
    };
    let void = matches!(end.reason, EndReason::Void | EndReason::LedgerOverflow);
    let returns: Vec<UnitReturn> = if void {
        let document = ctx
            .w
            .module_state::<FieldBook>()
            .and_then(|book| book.snapshot(&site).cloned());
        snapshot_returns(ctx, &site, document.as_ref()).unwrap_or_default()
    } else {
        units.to_vec()
    };
    let unit_ids: Vec<ObjId> = ctx
        .w
        .module_state::<FieldBook>()
        .and_then(|book| book.hosted(&site))
        .map(|hosted| hosted.live.units.iter().map(|row| row.unit).collect())
        .unwrap_or_default();
    let left = demote_units(ctx, &site, &unit_ids, &returns);
    // The registry's rows, as the match's own finish lets them go.
    let pilots: Vec<ObjId> = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.pilots_of(task))
        .unwrap_or_default();
    if let Some(registry) = ctx
        .w
        .module_state_mut::<crate::arena_bridge::ArenaRegistry>()
    {
        for pilot in &pilots {
            registry.unbind_pilot(*pilot);
        }
        registry.untrack(task);
    }
    for pilot in pilots {
        crate::arena_bridge::clear_cockpit(ctx, pilot);
    }
    if let Some(tasks) = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        && let Some((handle, join)) = tasks.remove(task)
    {
        handle.signal_stop();
        drop(join);
    }
    let ordinal = ctx
        .w
        .module_state_mut::<FieldBook>()
        .and_then(|book| {
            book.fire.remove(&site);
            book.forget_snapshot(&site);
            book.live.remove(&site)
        })
        .map(|hosted| hosted.live.field.ordinal);
    super::record::delete(ctx, &site);
    if let Some(record) = site_record(ctx, &site) {
        super::admit::owe_close(ctx, &site, &record, Some(left));
    }
    let line = format!(
        "Field {} on {} has settled: the ground there is the surface's again.",
        ordinal.unwrap_or(0),
        super::admit::site_name(ctx, &site)
    );
    super::tell_site(ctx, &site, &line);
    tracing::info!(
        site = %site,
        reason = ?end.reason,
        arena_ms = end.t_ms,
        "tacsim: an IC field demoted"
    );
}

/// **Check `units` in from a field and stand them on the surface** (this module's head),
/// answering the site record's new unit index. A unit with no return goes home as it went out.
pub(crate) fn demote_units(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    units: &[ObjId],
    returns: &[UnitReturn],
) -> Vec<LeftUnit> {
    let now = ctx.w.world_ic_now().at;
    let domain = super::parties::site_domain(ctx.w, site);
    let epochs: Vec<(ObjId, Option<crate::arena::mind::SortieId>)> = ctx
        .w
        .module_state::<FieldBook>()
        .and_then(|book| book.hosted(site))
        .map(|hosted| {
            hosted
                .live
                .units
                .iter()
                .map(|row| (row.unit, row.epoch))
                .collect()
        })
        .unwrap_or_default();
    let mut left = Vec::new();
    for unit in units {
        let back = returns.iter().find(|ret| ret.unit == *unit);
        let before = mush_tactics::module::unit_of(ctx.w, *unit).cloned();
        let (plan, state) = match (back, before.as_ref()) {
            (Some(back), Some(before)) => {
                match crate::field_ops::apply_ic_combat_return(before, back, None, now) {
                    Ok((_, assessment, plan)) => (plan, Some(assessment.state)),
                    Err(error) => {
                        tracing::warn!(unit = unit.0, %error, "tacsim: a field return's IC plan did not form; the unit checks in unchanged");
                        (None, None)
                    }
                }
            }
            _ => (None, None),
        };
        // Never the snapshot (Q-CF-35): an empty plan when nothing changed.
        let empty = mush_tactics::UnitMutationPlan::new(
            Vec::new(),
            None,
            Some(smol_str::SmolStr::new_static("field:checkin")),
        )
        .ok();
        let applied = match plan.as_ref().or(empty.as_ref()) {
            Some(plan) => mush_tactics::custody::check_in(ctx, *unit, Some(plan)),
            None => Err(mush_tactics::custody::CustodyError::Unavailable),
        };
        if let Err(error) = applied {
            tracing::error!(unit = unit.0, %error, "tacsim: a field unit's check-in refused; its marker is cleared and its cells stand");
            mush_sim::deployed::clear(ctx.w, *unit);
        }
        let pose = back.map(|back| back.pose.clone());
        if let (Some(pose), Some(domain)) = (pose.clone(), domain)
            && mush_sim::pose_of(ctx.w, *unit).is_none_or(|(_, at)| {
                at.x_cm != pose.x_cm || at.y_cm != pose.y_cm || at.heading_mdeg != pose.heading_mdeg
            })
            && let Err(error) = mush_sim::place(ctx, *unit, domain, pose)
        {
            tracing::warn!(unit = unit.0, %error, "tacsim: a field unit could not be stood at its field pose");
        }
        let condition = match state {
            Some(OperationalState::Wreck | OperationalState::Irrecoverable) => {
                Some(LeftCondition::Wreck)
            }
            Some(
                OperationalState::Immobile | OperationalState::Unarmed | OperationalState::Disabled,
            ) => Some(LeftCondition::Casualty),
            _ if back.is_some_and(|back| back.wreck) => Some(LeftCondition::Wreck),
            _ if back.is_some_and(|back| back.disabled) => Some(LeftCondition::Casualty),
            _ => None,
        };
        let Some(condition) = condition else {
            continue;
        };
        let Some((_, at)) = mush_sim::pose_of(ctx.w, *unit) else {
            continue;
        };
        let pose = SitePose {
            x_cm: at.x_cm,
            y_cm: at.y_cm,
            heading_mdeg: at.heading_mdeg,
        };
        if !pose.on_site() {
            continue;
        }
        left.push(LeftUnit {
            unit: *unit,
            pose,
            condition,
            epoch: epochs
                .iter()
                .find(|(id, _)| id == unit)
                .and_then(|(_, epoch)| *epoch),
            left_ic: now,
        });
    }
    left
}
