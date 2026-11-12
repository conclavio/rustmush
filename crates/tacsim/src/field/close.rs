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
//! **The absent** (doc 45 cut AP.1; Q-45-17): the site's settling line is said to whoever is
//! there; every player with a unit or a carrier in the field, connected or not, is told it
//! settled and what became of each of their units, and is sent a letter for a loss
//! (`super::notice`).
//!
//! **A void** (a panic, a stall, a ledger overflow, a refused resume) demotes from **the last
//! snapshot the world holds**: its units' cells and poses are the return. With no snapshot every
//! unit checks in as it went out, which is what the snapshot would have said.

use std::collections::BTreeSet;

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
    /// **The bay rigs a wrecked carrier took down while their claims held them** (doc 38a
    /// F-23.1): each owed its claim's `Destroyed` Close by the field's `close`, and its wreck
    /// once the claim lets it go.
    pub wrecked: Vec<ObjId>,
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
    // Doc 45 cut AP.1 (Q-45-17): what became of each stakeholder's units, read while the host
    // still holds the field and now its units and crews are checked in.
    let closing = super::notice::closing(ctx, &site, &demoted);
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
    // Doc 45 cut AP.1: and every player with a unit or a carrier in it, connected or not.
    super::notice::tell_closed(ctx, &site, &line, closing);
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
    let mut wrecked = Vec::new();
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
            wrecked.extend(wreck_bay_rigs(ctx, *unit));
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
            // Doc 38a F-03: a hull its claim still holds in its hangar cannot be unplaced (the
            // exclusion refuses it until the claim's `Destroyed` Close lands), so it is owed the
            // unplace, which the host's tick lands once the claim lets it go.
            if owes_unplace(&crate::minds::ic_sortie::claim_of(ctx.w, *unit)) {
                if let Some(owed) = ctx.w.module_state_mut::<OwedUnplaces>() {
                    owed.units.insert(*unit);
                }
            } else if mush_sim::pose_of(ctx.w, *unit).is_some()
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
        // Doc 41 cut RS.9a (the owner's ruling of 2026-11-05): a carrier wrecked while docked is
        // set down on her pad cell, where a tug can reach her, and indexed there.
        let set_down = (carrier && condition == LeftCondition::Wreck)
            .then(|| crate::surface::tow::set_down_on_pad(ctx, *unit))
            .flatten();
        let at = match (set_down, back.filter(|_| carrier)) {
            (Some(pose), _) => pose,
            (None, Some(back)) => back.pose.clone(),
            (None, None) => {
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
    // Doc 42b cut SU.3 (Q-42b-12): a yielded raider hull stands on the ground as a casualty.
    super::raiders::keep_yielded(ctx, site, &mut left, side);
    Demoted {
        left,
        remains,
        minds,
        wrecked,
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

/// **The set-downs the host owes the site records** (doc 41's RS.9a follow-up): per site, each
/// carrier wrecked while docked outside a field, the character who submits her `set_down` (her
/// helm holder) and the row it indexes. RAM: rebuilt by the tick from the hulls themselves.
#[derive(Clone, Debug, Default)]
pub(crate) struct OwedSetDowns(
    pub(crate)  std::collections::BTreeMap<
        CanonicalId,
        std::collections::BTreeMap<ObjId, (ObjId, LeftUnit)>,
    >,
);

/// **Set down every carrier wrecked while docked at a site, outside a field** (doc 41's RS.9a
/// follow-up; the owner's ruling of 2026-11-05; the host's tick): a hull loss no field checked
/// in — a hazard, a staff hand's damage, any later damage path — leaves the carrier on her pad as
/// the field's close does ([`demote_units`]): set down on her pad cell
/// ([`crate::surface::tow::set_down_on_pad`]), every rig in her bay a wreck with her
/// ([`wreck_bay_rigs`]), and indexed on the site's record as a wreck by the field action's
/// `set_down`, submitted as her helm holder, so a tug and the house arm read her like any
/// casualty. A carrier a running field holds is the close's; one already indexed is done.
pub(crate) fn set_down_docked_wrecks(ctx: &mut Ctx<'_>) {
    use crate::port::ship::{Berth, berth_of};
    let wrecked: Vec<ObjId> = crate::surface::recovery::world_hulls(ctx.w)
        .into_iter()
        .filter(|hull| {
            matches!(berth_of(ctx.w, *hull), Berth::Docked { def, .. } if def.site.is_some())
        })
        .filter(|hull| {
            crate::carrier_damage::damage_of(ctx.w, *hull)
                .is_some_and(|damage| damage.has(crate::carrier_damage::CarrierLoss::Wrecked))
        })
        .collect();
    let mut owed: std::collections::BTreeMap<
        CanonicalId,
        std::collections::BTreeMap<ObjId, (ObjId, LeftUnit)>,
    > = std::collections::BTreeMap::new();
    for carrier in wrecked {
        let Some((site, ..)) = crate::surface::tick::hull_view_berth(ctx.w, carrier) else {
            continue;
        };
        let in_field = ctx
            .w
            .module_state::<FieldBook>()
            .and_then(|book| book.hosted(&site))
            .is_some_and(|hosted| hosted.live.units.iter().any(|row| row.unit == carrier));
        if in_field
            || site_record(ctx, &site)
                .is_some_and(|record| record.units.iter().any(|row| row.unit == carrier))
        {
            continue;
        }
        let Some(actor) = crate::port::coop::helm_holder(ctx.w, carrier) else {
            tracing::warn!(
                carrier = carrier.0,
                "tacsim: a wrecked docked carrier has no helm holder to set her down"
            );
            continue;
        };
        let Some(pose) = crate::surface::tow::set_down_on_pad(ctx, carrier) else {
            continue;
        };
        let _ = wreck_bay_rigs(ctx, carrier);
        owed.entry(site).or_default().insert(
            carrier,
            (
                actor,
                LeftUnit {
                    unit: carrier,
                    pose: SitePose {
                        x_cm: pose.x_cm,
                        y_cm: pose.y_cm,
                        heading_mdeg: pose.heading_mdeg,
                    },
                    condition: LeftCondition::Wreck,
                    epoch: None,
                    left_ic: ctx.w.world_ic_now().at,
                },
            ),
        );
    }
    let Some(action) = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .and_then(|handles| handles.field.clone())
    else {
        return;
    };
    if let Some(book) = ctx.w.module_state_mut::<OwedSetDowns>() {
        book.0.clone_from(&owed);
    }
    let now = ctx.w.world_ic_now().at;
    for (site, rows) in owed {
        for (carrier, (actor, _)) in rows {
            if ctx.authoritative_pending(actor) {
                continue;
            }
            let Some(key) = super::provider::field_key(actor, &site, "set_down", carrier.0, now)
            else {
                continue;
            };
            let saved = ctx.actor;
            ctx.actor = actor;
            ctx.submit_authoritative(
                action.clone(),
                mush_core::PerceivedRef::SelfActor,
                Vec::new(),
                serde_json::json!({ "site": site.to_string(), "op": "set_down", "unit": carrier.0 }),
                key,
            );
            ctx.actor = saved;
        }
    }
}

/// **A wrecked carrier's bay** (doc 36 cut GC.3; Q-36-8): when `carrier`'s checked-in record
/// says its hull fell, every rig in its bay becomes a wreck where the carrier stands — the
/// arena's wreck mark written onto its own record ([`crate::carrier_damage::wreck_plan`]). The
/// rigs stay in the bay: the bay is in the wreck.
///
/// Doc 38a F-23.1: a rig whose claim still holds it in this bay cannot be mutated (the
/// exclusion's fence), so it is answered instead: owed its claim's `Destroyed` Close and, once
/// the claim lets it go, its wreck ([`land_owed_unplaces`]).
fn wreck_bay_rigs(ctx: &mut Ctx<'_>, carrier: ObjId) -> Vec<ObjId> {
    let mut held = Vec::new();
    if !crate::carrier_damage::damage_of(ctx.w, carrier)
        .is_some_and(|damage| damage.has(crate::carrier_damage::CarrierLoss::BayRigsWrecked))
    {
        return held;
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
        if owes_unplace(&crate::minds::ic_sortie::claim_of(ctx.w, rig)) {
            held.push(rig);
            if let Some(owed) = ctx.w.module_state_mut::<OwedUnplaces>() {
                owed.wrecks.insert(rig, carrier);
            }
            continue;
        }
        wreck_rig(ctx, carrier, rig);
    }
    held
}

/// Apply the wreck plan to the bay rig `rig` of the wrecked `carrier`.
fn wreck_rig(ctx: &mut Ctx<'_>, carrier: ObjId, rig: ObjId) -> bool {
    let Some(plan) = mush_tactics::module::unit_of(ctx.w, rig).and_then(|cells| {
        crate::carrier_damage::wreck_plan(crate::carrier_damage::spec_of(ctx.w, cells), cells)
    }) else {
        return true;
    };
    match mush_tactics::module::apply_plan(ctx, rig, &plan) {
        Ok(_) => {
            tracing::info!(
                carrier = carrier.0,
                rig = rig.0,
                "tacsim: a wrecked carrier's bay rig is a wreck"
            );
            true
        }
        Err(error) => {
            tracing::warn!(carrier = carrier.0, rig = rig.0, %error, "tacsim: a wrecked carrier's bay rig could not be wrecked");
            false
        }
    }
}

/// **The irrecoverable hulls owed their unplacing** (doc 38a F-03): each claimed hull a field
/// checked in irrecoverable while its claim still held it, its remains already on the record.
/// RAM, rebuilt at the first tick after a boot from every lane site's remains that still name a
/// hull standing on the surface ([`land_owed_unplaces`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct OwedUnplaces {
    units: BTreeSet<ObjId>,
    /// Doc 38a F-23.1: the bay rigs owed their wreck once their claims let them go. RAM only: a
    /// restart before the claim's Close lands loses the wreck (the claim still closes).
    wrecks: std::collections::BTreeMap<ObjId, ObjId>,
    scanned: bool,
}

/// Whether a hull whose claim reads `claim` must wait to be unplaced: its claim still holds it
/// in its hangar (doc 38a F-03).
pub(crate) const fn owes_unplace(claim: &crate::minds::ic_sortie::ClaimRead) -> bool {
    matches!(claim, crate::minds::ic_sortie::ClaimRead::Active { .. })
}

/// **Unplace every owed hull its claim has let go** (doc 38a F-03; the host's tick): a hull still
/// held waits; one no longer on the surface is forgotten.
pub(crate) fn land_owed_unplaces(ctx: &mut Ctx<'_>) {
    let scan = ctx
        .w
        .module_state::<OwedUnplaces>()
        .is_some_and(|owed| !owed.scanned);
    if scan {
        let mut found = BTreeSet::new();
        for site in crate::surface::chart::lane_sites(ctx.w) {
            let Some(record) = site_record(ctx, &site.entry) else {
                continue;
            };
            for object in &record.objects {
                if let crate::surface::site_record::GroundKind::Remains { of: Some(unit), .. } =
                    object.kind
                    && mush_sim::pose_of(ctx.w, unit).is_some()
                {
                    found.insert(unit);
                }
            }
        }
        if let Some(owed) = ctx.w.module_state_mut::<OwedUnplaces>() {
            owed.units.extend(found);
            owed.scanned = true;
        }
    }
    let units: Vec<ObjId> = ctx
        .w
        .module_state::<OwedUnplaces>()
        .map(|owed| owed.units.iter().copied().collect())
        .unwrap_or_default();
    for unit in units {
        if owes_unplace(&crate::minds::ic_sortie::claim_of(ctx.w, unit)) {
            continue;
        }
        if mush_sim::pose_of(ctx.w, unit).is_some()
            && let Err(error) =
                mush_sim::unplace(ctx, unit, "tacsim: an irrecoverable hull leaves remains")
        {
            tracing::warn!(unit = unit.0, %error, "tacsim: an owed irrecoverable hull could not be taken off the surface");
            continue;
        }
        if let Some(owed) = ctx.w.module_state_mut::<OwedUnplaces>() {
            owed.units.remove(&unit);
        }
    }
    let wrecks: Vec<(ObjId, ObjId)> = ctx
        .w
        .module_state::<OwedUnplaces>()
        .map(|owed| owed.wrecks.iter().map(|(rig, by)| (*rig, *by)).collect())
        .unwrap_or_default();
    for (rig, carrier) in wrecks {
        if owes_unplace(&crate::minds::ic_sortie::claim_of(ctx.w, rig)) {
            continue;
        }
        if wreck_rig(ctx, carrier, rig)
            && let Some(owed) = ctx.w.module_state_mut::<OwedUnplaces>()
        {
            owed.wrecks.remove(&rig);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Doc 38a F-03: an irrecoverable hull its claim still holds waits for its unplacing.** The
    /// exclusion refuses any unplacing of a held hull until the claim's `Destroyed` Close lands,
    /// so the check-in owes it; an unclaimed hull is unplaced at once.
    #[test]
    fn f03_a_held_irrecoverable_hull_is_owed_its_unplacing() {
        assert!(owes_unplace(&crate::minds::ic_sortie::ClaimRead::Active {
            open: mush_core::CommitToken::from_bytes([7; 16]),
            dispatcher: ObjId(42),
        }));
        assert!(!owes_unplace(&crate::minds::ic_sortie::ClaimRead::None));
    }
}
