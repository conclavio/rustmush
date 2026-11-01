//! **The carrier as a unit** (doc 32 cut CF.3; Q-CF-19, Q-CF-38, §4a; doc 27 Q-H10, FO-13).
//!
//! A programme hull settled on a promoted site's pad is on the ground like everyone else
//! (Q-CF-16): it is admitted at its pad pose with its real sections — the Dray, the Dray-P and
//! the Halcyon carry unit specs with sections and facet bands — through the same custody
//! check-out, and its damage comes home through the same IC check-in into the hull's own unit
//! record, which `+ship` reads in port (Q-CF-19). It is shootable from the first pulse; what a
//! hit does to lift, sailing, the bay and the hold is doc 32 F-3, so in this cut a hit is section
//! damage only.
//!
//! - **Its party** is the roster it launched (§3 Q-CF-4's note): the activation of any open DT
//!   flight on this site whose carrier it is, else a party of one led by whoever commands it
//!   ([`crate::port::coop::helm_holder`]). One commander behind the carrier and its rigs makes
//!   them Friends through the field's standing-friendship rule (Q-CF-5).
//! - **Its crew** (Q-CF-38): everyone physically aboard at admission is "aboard the carrier"
//!   for the field's life, listed on the carrier's row of the live record; doc 32 cut CF.5's crew
//!   state reads those rows (`crate::field::crew::crew_location`) and sets each soul's record
//!   from them at the carrier's check-in.
//! - **It is never towed** (FO-13): the twice-own-tonnage rule already puts a carrier beyond any
//!   tug, and [`refuse_tow`] says so by name before the tonnage is read.
//! - **It departs mid-field only live, on the leave-behind confirmation** (Q-H10; doc 32 F-6,
//!   ruled 2026-10-31): a live `+ship/lift` naming every unit out removes it from the field
//!   outright (`crate::field::exit`); [`departure_refusal`] is a queued departure's gate.

use mush_core::{Ctx, ObjId, World};

use super::FieldBook;

/// **A queued departure while the carrier is in a field** (Q-H10, doc 32 §4a, F-6): its units
/// are out on a promoted site, so a departure leaves them; `{name}`, `{site}`, `{units}`.
pub const DEPART_IN_FIELD: &str = "The {name} is in the field on {site}: lifting now leaves \
     {units} on the ground. A queued departure never takes a carrier out of a field; lift live \
     with leave= naming every unit you have out (doc 32 F-6).";

/// **A tow naming a carrier** (FO-13): `{name}`.
pub const TOW_CARRIER: &str =
    "The {name} is a carrier: past twice any tug's own tonnage, it is never towed.";

/// Whether `unit` is a programme hull — a carrier (Dray, Dray-P, Halcyon).
#[must_use]
pub fn is_carrier(w: &World, unit: ObjId) -> bool {
    crate::port::ship::hull_view(w, unit).is_some()
}

/// **Everyone physically aboard `carrier` now**: the souls in its bounded subtree (rooms, bay,
/// a seated operator's placement), by id.
#[must_use]
pub(crate) fn souls_aboard(w: &World, carrier: ObjId) -> Vec<ObjId> {
    let Some(airlock) = crate::port::ship::hull_view(w, carrier).and_then(|view| view.airlock())
    else {
        return Vec::new();
    };
    crate::port::voyage::souls_aboard(w, airlock)
}

/// **The carrier's open hull epoch** (doc 23 S.2), through the command lens.
pub(crate) fn open_epoch(ctx: &Ctx<'_>, carrier: ObjId) -> Option<crate::arena::mind::SortieId> {
    let handles = ctx.w.module_state::<crate::port::families::PortHandles>()?;
    ctx.action_state(
        &handles.families.epochs,
        &crate::port::epoch::EpochKey(carrier),
    )
    .ok()
    .flatten()
    .and_then(|record| record.open.as_ref().map(|open| open.id))
}

/// **How far from the pad's cell a carrier may be stood**, cells (Chebyshev): several hulls
/// can be settled on one pad, and the kernel stands one unit in a cell.
pub const PAD_SPREAD_CELLS: u16 = 8;

/// **Each carrier's pad pose on the field's ground** (Q-CF-19), in unit order.
///
/// A settled hull's own placement is the domain's passive pose, not the pad the surface draws
/// ([`crate::surface::excursion::PAD_CENTRE_CM`]), so the field stands it there: the pad's cell
/// for the first carrier, then the nearest cell passable to it and free of every other unit
/// (`taken`), nearest ring first, row-major within a ring. Its heading is its own. The hull's
/// surface placement is never moved: a carrier stands still in a field (its spec's top speed
/// is nil) and goes home where it was.
///
/// # Errors
///
/// A carrier with no unit record or no placement, or no free cell within
/// [`PAD_SPREAD_CELLS`] of the pad.
pub(crate) fn pad_poses(
    w: &World,
    terrain: &crate::arena::state::ArenaTerrain,
    carriers: &[ObjId],
    taken: &mut std::collections::BTreeSet<(u16, u16)>,
) -> Result<std::collections::BTreeMap<ObjId, mush_sim::data::Pose>, String> {
    let pad = crate::surface::excursion::PAD_CENTRE_CM;
    let (px, py) = terrain
        .cell_at(pad, pad)
        .ok_or_else(|| "the pad is off the field's ground".to_owned())?;
    let side = i64::from(terrain.cell_cm());
    let mut poses = std::collections::BTreeMap::new();
    for carrier in carriers {
        let cells = mush_tactics::module::unit_of(w, *carrier)
            .ok_or_else(|| format!("#{} has no tactical unit record", carrier.0))?;
        let row = crate::arena::state::CostRow::read(cells);
        let (_, own) = mush_sim::pose_of(w, *carrier)
            .ok_or_else(|| format!("#{} is not placed", carrier.0))?;
        let cell = (0..=PAD_SPREAD_CELLS)
            .flat_map(|ring| ring_cells((px, py), ring))
            .find(|cell| !taken.contains(cell) && terrain.passable_for(row, cell.0, cell.1))
            .ok_or_else(|| {
                format!(
                    "the pad has no free ground for #{} within {PAD_SPREAD_CELLS} cells",
                    carrier.0
                )
            })?;
        taken.insert(cell);
        let mut pose = own;
        pose.x_cm = i64::from(cell.0) * side + side / 2;
        pose.y_cm = i64::from(cell.1) * side + side / 2;
        poses.insert(*carrier, pose);
    }
    Ok(poses)
}

/// **Where a unit launched down a carrier's ramp stands** (doc 32 §17, the fix for G-4's
/// "two hulls on one pad launch their rigs onto one cell"): the ramp's own pose while no other
/// unit stands in the ramp's cell, else the first cell one cell from it (row-major) that no
/// other unit on the site's ground stands in and that `unit` can stand on, at the ramp's pose
/// moved by exactly one cell on each axis that differs — so the pose is never more than one cell
/// from the ramp, and every "beside the ramp" test (a launch's finish, a DT flight's
/// `at_ramp`, the recover, CF.5's exit) reads it as at the ramp. A field holds one unit a cell,
/// and every hull on one pad shares its ramp, so without this a second rig launched onto a ramp
/// cell already taken stalled every field on the site. With no free neighbour (or a ground that
/// does not build) the ramp pose stands, as before.
#[must_use]
pub(crate) fn launch_pose(
    ctx: &Ctx<'_>,
    site: &mush_core::CanonicalId,
    domain: mush_sim::DomainId,
    ramp: &mush_sim::data::Pose,
    unit: ObjId,
) -> mush_sim::data::Pose {
    use crate::surface::excursion::site_cell;
    let w: &World = ctx.w;
    let taken: std::collections::BTreeSet<(i64, i64)> = super::parties::on_ground(w, domain)
        .into_iter()
        .filter(|other| *other != unit)
        .filter_map(|other| mush_sim::pose_of(w, other))
        .filter(|(on, _)| *on == domain)
        .map(|(_, pose)| site_cell(pose.x_cm, pose.y_cm))
        .collect();
    let at = site_cell(ramp.x_cm, ramp.y_cm);
    if !taken.contains(&at) {
        return ramp.clone();
    }
    let Some(row) = mush_tactics::module::unit_of(w, unit).map(crate::arena::state::CostRow::read)
    else {
        return ramp.clone();
    };
    let record = super::close::site_record(ctx, site)
        .unwrap_or_else(|| crate::surface::site_record::SiteRecord::new(site.clone()));
    let Ok(ground) = super::ground::definition(w, site, &record, Vec::new()) else {
        return ramp.clone();
    };
    let (Ok(cx), Ok(cy)) = (u16::try_from(at.0), u16::try_from(at.1)) else {
        return ramp.clone();
    };
    let side = i64::from(crate::surface::ground::SITE_CELL_CM);
    ring_cells((cx, cy), 1)
        .into_iter()
        .find(|cell| {
            !taken.contains(&(i64::from(cell.0), i64::from(cell.1)))
                && ground.terrain.passable_for(row, cell.0, cell.1)
        })
        .map_or_else(
            || ramp.clone(),
            |cell| {
                let mut pose = ramp.clone();
                pose.x_cm += (i64::from(cell.0) - at.0) * side;
                pose.y_cm += (i64::from(cell.1) - at.1) * side;
                pose
            },
        )
}

/// The cells `ring` cells from `centre` (Chebyshev), row-major; `ring 0` is the centre.
pub(crate) fn ring_cells(centre: (u16, u16), ring: u16) -> Vec<(u16, u16)> {
    let (cx, cy) = (i32::from(centre.0), i32::from(centre.1));
    let r = i32::from(ring);
    let mut out = Vec::new();
    for y in (cy - r)..=(cy + r) {
        for x in (cx - r)..=(cx + r) {
            if (x - cx).abs().max((y - cy).abs()) != r {
                continue;
            }
            if let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) {
                out.push((x, y));
            }
        }
    }
    out
}

/// **The departure gate** (Q-H10): `Some(line)` when `hull` is held by a running field. The
/// gate of a queued departure, which never presumes the confirmation; a live one takes the
/// carrier out of the field instead (doc 32 F-6). The line names the units it would leave.
#[must_use]
pub fn departure_refusal(w: &World, hull: ObjId) -> Option<String> {
    let site = super::field_of_unit(w, hull)?;
    let left: Vec<String> = w
        .module_state::<FieldBook>()
        .and_then(|book| book.hosted(&site))
        .map(|hosted| {
            hosted
                .live
                .units
                .iter()
                .filter(|row| row.unit != hull)
                .map(|row| crate::port::coop::name_of(w, row.unit))
                .collect()
        })
        .unwrap_or_default();
    let units = if left.is_empty() {
        "nobody".to_owned()
    } else {
        left.join(", ")
    };
    Some(
        DEPART_IN_FIELD
            .replace("{name}", &crate::port::coop::name_of(w, hull))
            .replace("{site}", &crate::surface::act::site_name(w, &site))
            .replace("{units}", &units),
    )
}

/// **The tow gate** (FO-13): `Some(line)` when `target` is a carrier.
#[must_use]
pub fn refuse_tow(w: &World, target: ObjId) -> Option<String> {
    is_carrier(w, target)
        .then(|| TOW_CARRIER.replace("{name}", &crate::port::coop::name_of(w, target)))
}
