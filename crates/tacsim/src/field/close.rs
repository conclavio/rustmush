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
//! wreck a **wreck** (Q-CF-25).
//!
//! **Close-destroyed** (doc 36 cut GC.3; Q-36-18, Q-CF-25 c): an `Irrecoverable` hull is no
//! index entry. It leaves the surface, and the close owes the record a **remains** object with
//! its mass where it fell ([`crate::surface::site_record::LeftRemains`]); the close's act ends its
//! epoch `Destroyed` in the same bundle (`super::provider`). A carrier is never remains in this
//! cut: its hull's loss is a wreck, by the carrier's loss table ([`crate::carrier_damage`]), and
//! a wrecked carrier's bay rigs become wrecks where it stands.
//!
//! **Standing** (doc 36 cut GC.10; Q-36-11): at demotion the fire each party observed moves the
//! standing of the shooter's principals with its victims' factions, and friendly fire's change
//! is offered to the shooter party's leader to void (`super::standing`). A void end writes none.
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
use crate::surface::site_record::{LeftCondition, LeftRemains, LeftUnit, SitePose, SiteRecord};

/// **What a demotion leaves on the site** ([`demote_units`]): the record's new unit index and
/// the remains of every irrecoverable hull (doc 36 cut GC.3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Demoted {
    /// The casualties and wrecks, the record's new unit index.
    pub left: Vec<LeftUnit>,
    /// The remains the close drops, each closing its hull's epoch destroyed.
    pub remains: Vec<LeftRemains>,
    /// **The minds checked in** (doc 23 cut D-23-18): one row per licensed hull checked in on an
    /// open epoch, owed to its rig record by the field's next write on the site.
    pub minds: Vec<super::record::CarriedRow>,
}

/// **The units the last snapshot holds**, as returns (the void rule), when it decodes on the
/// field's own ground (its `homes` and its `held` walls from its live record, doc 32 cut CF.6b;
/// doc 36a §11 #1).
pub(crate) fn snapshot_returns(
    ctx: &Ctx<'_>,
    site: &CanonicalId,
    document: Option<&serde_json::Value>,
    homes: Vec<crate::arena::state::EntryZone>,
    held: &[super::record::HeldWall],
) -> Option<Vec<UnitReturn>> {
    let document = document?;
    let record = site_record(ctx, site)?;
    let ground = super::admit::ground_with_held(ctx.w, site, &record, homes, held).ok()?;
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
        let (homes, held) = ctx
            .w
            .module_state::<FieldBook>()
            .and_then(|book| book.hosted(&site))
            .map(|hosted| (hosted.live.homes.clone(), hosted.live.held.clone()))
            .unwrap_or_default();
        snapshot_returns(ctx, &site, document.as_ref(), homes, &held).unwrap_or_default()
    } else {
        units.to_vec()
    };
    let unit_ids: Vec<ObjId> = ctx
        .w
        .module_state::<FieldBook>()
        .and_then(|book| book.hosted(&site))
        .map(|hosted| hosted.live.units.iter().map(|row| row.unit).collect())
        .unwrap_or_default();
    let mut demoted = demote_units(ctx, &site, &unit_ids, &returns);
    // Doc 36 cut GC.10 (Q-36-11): the standing the field's fire moved, and the leaders' void of
    // friendly fire, read while the host still holds the parties.
    super::standing::at_demotion(ctx, &site, void);
    // Doc 32 cut CF.4 (Q-CF-28): what each party saw, read while the host still holds the field.
    let learned = super::known::learned_at_close(ctx, task, &site, &returns, &demoted);
    super::known::forget(ctx, &site);
    // Doc 32 cut CF.5 (Q-CF-38): every pilot's crew state, from the checked-in hulls, and every
    // soul a carrier's row listed aboard (CF.3).
    let seats: Vec<(ObjId, Option<ObjId>, Vec<ObjId>)> = ctx
        .w
        .module_state::<FieldBook>()
        .and_then(|book| book.hosted(&site))
        .map(|hosted| {
            hosted
                .live
                .units
                .iter()
                .map(|row| {
                    (
                        row.unit,
                        row.pilot.as_ref().map(|(pilot, _)| *pilot),
                        row.aboard.clone(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    for (unit, pilot, aboard) in seats {
        super::crew::check_in(ctx, &site, unit, pilot);
        super::crew::check_in_aboard(ctx, &site, unit, &aboard);
    }
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
        .map(|hosted| {
            // Doc 23 cut D-23-18: a mind checked in at the ramp whose `exit` has not yet laid it
            // on its epoch record rides the close instead.
            carry_unwritten(&mut demoted.minds, hosted.live.carried);
            hosted.live.field.ordinal
        });
    super::record::delete(ctx, &site);
    if let Some(record) = site_record(ctx, &site) {
        super::admit::owe_close(ctx, &site, &record, Some(demoted), learned);
    }
    let line = super::FIELD_SETTLED
        .replace("{n}", &ordinal.unwrap_or(0).to_string())
        .replace("{site}", &super::admit::site_name(ctx, &site));
    super::tell_site(ctx, &site, &line);
    tracing::info!(
        site = %site,
        reason = ?end.reason,
        arena_ms = end.t_ms,
        "tacsim: an IC field demoted"
    );
}

/// **What a unit leaves on the site record** (Q-CF-25): an immobile, unarmed or disabled hull a
/// casualty, a wreck or a total loss a wreck, else nothing — from its IC assessment, or the
/// kernel's own flags when none formed.
fn left_condition(
    state: Option<OperationalState>,
    back: Option<&UnitReturn>,
) -> Option<LeftCondition> {
    match state {
        Some(OperationalState::Wreck | OperationalState::Irrecoverable) => {
            Some(LeftCondition::Wreck)
        }
        Some(
            OperationalState::Immobile | OperationalState::Unarmed | OperationalState::Disabled,
        ) => Some(LeftCondition::Casualty),
        _ if back.is_some_and(|back| back.wreck) => Some(LeftCondition::Wreck),
        _ if back.is_some_and(|back| back.disabled) => Some(LeftCondition::Casualty),
        _ => None,
    }
}

/// **What `back` would leave on the site record if the field demoted now** (doc 36 cut GC.0):
/// [`demote_units`]'s rule over a return, applied to nothing.
pub(crate) fn condition_now(ctx: &Ctx<'_>, back: &UnitReturn) -> Option<LeftCondition> {
    let now = ctx.w.world_ic_now().at;
    let state = mush_tactics::module::unit_of(ctx.w, back.unit).and_then(|before| {
        crate::field_ops::apply_ic_combat_return(before, back, None, now)
            .ok()
            .map(|(_, assessment, _)| assessment.state)
    });
    left_condition(state, Some(back))
}

/// **Check `units` in from a field and stand them on the surface** (this module's head),
/// answering the site record's new unit index and the remains it owes. A unit with no return
/// goes home as it went out.
pub(crate) fn demote_units(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    units: &[ObjId],
    returns: &[UnitReturn],
) -> Demoted {
    let now = ctx.w.world_ic_now().at;
    let domain = super::parties::site_domain(ctx.w, site);
    // Doc 32 cut S.0: the record's own side, which its demote holds every pose to.
    let side = site_record(ctx, site).map_or_else(
        || crate::surface::site_record::site_side_or_default(ctx.w, site),
        |record| record.side,
    );
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
    let mut remains = Vec::new();
    let mut minds = Vec::new();
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
        // Doc 23 cut D-23-18: a licensed hull's mind, carried on its epoch's record.
        if let Some(row) = back.and_then(|back| carried_row(ctx, back)) {
            minds.push(row);
        }
        // CF.3: a carrier never moved; its surface placement stands, and the record keeps the
        // pad pose it was fought at.
        let carrier = super::carrier::is_carrier(ctx.w, *unit);
        // Doc 36 cut GC.3 (Q-36-8): a carrier whose hull fell wrecks the rigs in its bay.
        if carrier {
            wreck_bay_rigs(ctx, *unit);
        }
        // Doc 40 (the ramp exit's boarding window; the owner's ruling, 2026-11-04): a raider's
        // fixture carrier checked in a wreck takes the hulls aboard it with it.
        super::raiders::wreck_aboard(ctx, *unit, units);
        // Doc 36 cut GC.3 (Q-36-18): an irrecoverable hull leaves remains where it fell, and the
        // surface no longer holds it.
        if state == Some(OperationalState::Irrecoverable) && !carrier {
            let at = back
                .map(|back| back.pose.clone())
                .or_else(|| mush_sim::pose_of(ctx.w, *unit).map(|(_, at)| at));
            if mush_sim::pose_of(ctx.w, *unit).is_some()
                && let Err(error) =
                    mush_sim::unplace(ctx, *unit, "tacsim: an irrecoverable hull leaves remains")
            {
                tracing::warn!(unit = unit.0, %error, "tacsim: an irrecoverable hull could not be taken off the surface");
            }
            let pose = at.map(|at| SitePose {
                x_cm: at.x_cm,
                y_cm: at.y_cm,
                heading_mdeg: at.heading_mdeg,
            });
            if let Some(pose) = pose.filter(|pose| pose.on_site(side)) {
                remains.push(LeftRemains {
                    unit: *unit,
                    pose,
                    mass_kg: crate::carrier_damage::mass_kg(ctx.w, *unit).unwrap_or(0),
                    epoch: epochs
                        .iter()
                        .find(|(id, _)| id == unit)
                        .and_then(|(_, epoch)| *epoch),
                    left_ic: now,
                });
            }
            continue;
        }
        let pose = back.map(|back| back.pose.clone()).filter(|_| !carrier);
        if let (Some(pose), Some(domain)) = (pose.clone(), domain)
            && mush_sim::pose_of(ctx.w, *unit).is_none_or(|(_, at)| {
                at.x_cm != pose.x_cm || at.y_cm != pose.y_cm || at.heading_mdeg != pose.heading_mdeg
            })
            && let Err(error) = mush_sim::place(ctx, *unit, domain, pose)
        {
            tracing::warn!(unit = unit.0, %error, "tacsim: a field unit could not be stood at its field pose");
        }
        let Some(condition) = left_condition(state, back) else {
            continue;
        };
        let at = match back.filter(|_| carrier) {
            Some(back) => back.pose.clone(),
            None => {
                let Some((_, at)) = mush_sim::pose_of(ctx.w, *unit) else {
                    continue;
                };
                at
            }
        };
        let pose = SitePose {
            x_cm: at.x_cm,
            y_cm: at.y_cm,
            heading_mdeg: at.heading_mdeg,
        };
        if !pose.on_site(side) {
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
    // Doc 36 cut GC.4 (Q-36-20): every casualty the field carried as a tug's load stands at its
    // tug's cell, indexed as it was.
    crate::surface::tow::set_down(ctx, site, units, &mut left, side);
    // Doc 36a lane 3 (P0-8): a hull the field never held keeps the row an earlier field gave it.
    keep_held_out(ctx, site, units, &mut left, &remains, side);
    Demoted {
        left,
        remains,
        minds,
    }
}

/// **The mind a check-in carries for `back`** (doc 23 cut D-23-18, §17.7): a hull whose claim
/// the book holds active, on an open epoch, with a mind row that fits its class cap — the row
/// as the arena returned it, keyed by the rig record's own epoch. Anything else carries nothing:
/// a stock mind remembers nothing between fields.
fn carried_row(ctx: &Ctx<'_>, back: &UnitReturn) -> Option<super::record::CarriedRow> {
    let mind = back.mind.as_deref()?;
    if !matches!(
        crate::minds::ic_sortie::claim_of(ctx.w, back.unit),
        crate::minds::ic_sortie::ClaimRead::Active { .. }
    ) {
        return None;
    }
    let epoch = super::parties::rig_record(ctx, back.unit)?.sortie?.id;
    Some(super::record::CarriedRow {
        unit: back.unit,
        epoch,
        mind: mind.clone(),
    })
}

/// **Fold `unwritten` into `minds`** (doc 23 cut D-23-18): the ramp's check-ins still owed,
/// each kept unless the demotion checked the same hull in again (it never does: a hull out at
/// the ramp is no unit of the field).
pub(crate) fn carry_unwritten(
    minds: &mut Vec<super::record::CarriedRow>,
    unwritten: Vec<super::record::CarriedRow>,
) {
    for row in unwritten {
        if !minds.iter().any(|kept| kept.unit == row.unit) {
            minds.push(row);
        }
    }
}

/// **The rows of hulls that were never in the field** (doc 36a lane 3, P0-8): a hull held out of
/// the field under IC custody stood on the ground as a wall (`super::admit`, Q-36-44) and was no
/// unit of it, so the close's index — which replaces the record's — would have wiped the row an
/// earlier field gave it. Every indexed hull not among `units`, not already in `left` and not
/// remains, still standing on this site's ground, keeps its row at the pose it stands at. A row
/// for a hull no longer on the ground is stale and goes, as it always did.
fn keep_held_out(
    ctx: &Ctx<'_>,
    site: &CanonicalId,
    units: &[ObjId],
    left: &mut Vec<LeftUnit>,
    remains: &[LeftRemains],
    side: u16,
) {
    let Some(record) = site_record(ctx, site) else {
        return;
    };
    let Some(domain) = super::parties::site_domain(ctx.w, site) else {
        return;
    };
    for mut row in record.units {
        if units.contains(&row.unit)
            || left.iter().any(|kept| kept.unit == row.unit)
            || remains.iter().any(|lost| lost.unit == row.unit)
        {
            continue;
        }
        let Some((on, at)) = mush_sim::pose_of(ctx.w, row.unit) else {
            continue;
        };
        if on != domain {
            continue;
        }
        let pose = SitePose {
            x_cm: at.x_cm,
            y_cm: at.y_cm,
            heading_mdeg: at.heading_mdeg,
        };
        if pose.on_site(side) {
            row.pose = pose;
        }
        left.push(row);
    }
}

/// **Whether `unit`'s own record reads `Irrecoverable`** (doc 36 cut GC.3; Q-36-18): a hull that
/// left remains. It is off the ground but in no bay, so nothing may take it for a hull aboard.
pub(crate) fn is_irrecoverable(w: &mush_core::World, unit: ObjId) -> bool {
    mush_tactics::module::unit_of(w, unit)
        .and_then(|cells| crate::field_ops::assess_tacsim_unit(cells, None).ok())
        .is_some_and(|assessment| assessment.state == OperationalState::Irrecoverable)
}

/// **A wrecked carrier's bay** (doc 36 cut GC.3; Q-36-8): when `carrier`'s checked-in record
/// says its hull fell, every rig in its bay becomes a wreck where the carrier stands — the
/// arena's wreck mark written onto its own record ([`crate::carrier_damage::wreck_plan`]). The
/// rigs stay in the bay: the bay is in the wreck.
fn wreck_bay_rigs(ctx: &mut Ctx<'_>, carrier: ObjId) {
    if !crate::carrier_damage::damage_of(ctx.w, carrier)
        .is_some_and(|damage| damage.has(crate::carrier_damage::CarrierLoss::BayRigsWrecked))
    {
        return;
    }
    let rigs: Vec<ObjId> = crate::port::ship::hull_view(ctx.w, carrier)
        .map(|view| {
            crate::port::vehicle::rigs_in_bay(ctx.w, &view)
                .into_iter()
                .map(|rig| rig.tv)
                .collect()
        })
        .unwrap_or_default();
    for rig in rigs {
        let Some(plan) = mush_tactics::module::unit_of(ctx.w, rig).and_then(|cells| {
            crate::carrier_damage::wreck_plan(crate::carrier_damage::spec_of(ctx.w, cells), cells)
        }) else {
            continue;
        };
        match mush_tactics::module::apply_plan(ctx, rig, &plan) {
            Ok(_) => tracing::info!(
                carrier = carrier.0,
                rig = rig.0,
                "tacsim: a wrecked carrier's bay rig is a wreck"
            ),
            Err(error) => {
                tracing::warn!(carrier = carrier.0, rig = rig.0, %error, "tacsim: a wrecked carrier's bay rig could not be wrecked");
            }
        }
    }
}
