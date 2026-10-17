//! **The passage planner** (doc 18 cut IC9; doc 18 §4.4 "Passage: the planner
//! over the sky at departure", "The fuel function is the planner's output",
//! §4.8 "The seams the slice builds"; D-18-11, D-18-30, D-18-44 … D-18-47).
//!
//! Pure: no `World`, no `Ctx`, checked integer arithmetic throughout. One
//! planner, asked once, at departure:
//!
//! - [`plan_passage`] walks a lane route's legs as **body pairs** and, per leg,
//!   finds the intercept by a fixed number of passes ([`intercept_passes`],
//!   [`INTERCEPT_PASSES`]): the chord from the origin body at the leg's start to
//!   the target body at the leg's end, the unladen seconds at the drive's coast
//!   rate, the laden seconds through mush-sim's one laden step
//!   ([`mush_sim::laden_leg_durations`], `route_duration`'s floor arithmetic,
//!   lead ruling L1), and the end instant fed back. A leg between two places on
//!   one body uses its authored `base_s` through the same laden step.
//! - [`passage_burn`] is the drive law's fuel function (§4.4: the Coast law is
//!   distance-free, a fraction of departure mass, whole tonnes, lead ruling
//!   L19); [`crew_days`] is D-18-30's one rule; [`payload_kg`] is the payload
//!   the planner, `+ship/plan`, `+ship/undock` and the burn's prepare all sum
//!   the same way (lead ruling L12).
//! - [`choose_route`] plans every route a port lists for a destination and
//!   keeps the fastest, ties to the earlier listing (D-18-47); a listed route
//!   that cannot be planned refuses (lead ruling L22). `via` is reserved for the
//!   astronomy engine (post-slice cut P-A).
//!
//! The plan is frozen in the burn's ticket (D-18-46): nothing after the burn
//! re-reads the sky, and the journey flies the plan's own seconds. **No slice
//! indexing** anywhere in this file (tacsim's restriction census, reviewer F4):
//! legs are walked with iterators, `.first()` and `.last()`.

use mush_core::CanonicalId;
use mush_sim::{DistanceLaw, DriveProfile, RouteDefError};

use crate::definitions::GoodsTable;
use crate::pack::{PackEntry, RulesPack};
use crate::sky::Sky;

pub use crate::definitions::logistics::DriveLaw;
pub use crate::port::families::{LegPlan, PassagePlan};

/// How many fixed-point passes [`plan_passage`] makes per leg (§4.4).
pub const INTERCEPT_PASSES: usize = 3;

/// Seconds in one IC day: the crew-day rule's divisor (D-18-30).
pub const DAY_SECS: u32 = 86_400;

/// **A hull's drive, as the planner reads it** (§4.8): its class, the law the
/// class flies under, and the payload rail (`route_duration`'s laden
/// denominator).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HullDrive {
    /// The drive class a route's `drive_class` names (`coast_hauler`).
    pub class: CanonicalId,
    /// The law (`coast {kms 226, burn_permille 105}` for the Dray).
    pub law: DriveLaw,
    /// The payload rail, kilograms.
    pub payload_ceiling_kg: u64,
}

/// **Why a passage could not be planned** (§4.8's six, plus `ViaReserved` and
/// `SkyUnavailable`, lead ruling L10, plus `Unreadable` — the payload's own
/// read, see the cut's report).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// The pack has no route entry by that id.
    #[error("route {route} is not a route of the lane")]
    UnknownRoute {
        /// The id asked for.
        route: String,
    },
    /// The route is flown by another drive class.
    #[error("route {route} is flown by drive class {expected}, not {actual}")]
    UnknownDriveClass {
        /// The route.
        route: String,
        /// The class the route declares.
        expected: String,
        /// The hull's class.
        actual: String,
    },
    /// A place's body the sky does not answer for.
    #[error("{place}'s body is not in the sky")]
    UnknownBody {
        /// The place.
        place: String,
    },
    /// A place with no body: the sky cannot place it.
    #[error("{place} is on no body, so no leg to or from it can be planned")]
    NoSkyLeg {
        /// The place.
        place: String,
    },
    /// The payload is over the hull's rail.
    #[error("payload {payload_kg} kg is over the rated {ceiling_kg} kg")]
    Overloaded {
        /// The payload, kilograms.
        payload_kg: u64,
        /// The rail, kilograms.
        ceiling_kg: u64,
    },
    /// The arithmetic left its type (or produced a plan that fails its own
    /// check): never a wrap, never a panic.
    #[error("the passage arithmetic overflowed")]
    Overflow,
    /// A chosen waypoint (`via`) is reserved for the astronomy engine.
    #[error("a passage by a chosen waypoint is not offered")]
    ViaReserved,
    /// The sky could not state its revision.
    #[error("the sky cannot state its revision")]
    SkyUnavailable,
    /// A quantity the payload sums could not be read: a pool, or a good the
    /// goods table does not weigh.
    #[error("{what} could not be read")]
    Unreadable {
        /// What.
        what: String,
    },
}

/// mush-sim's laden-step refusals as the planner's.
fn route_error(error: RouteDefError) -> PlanError {
    match error {
        RouteDefError::Overloaded { payload, capacity } => PlanError::Overloaded {
            payload_kg: payload,
            ceiling_kg: capacity,
        },
        RouteDefError::NoLegs
        | RouteDefError::TooManyLegs { .. }
        | RouteDefError::ZeroLeg
        | RouteDefError::HazardBand { .. }
        | RouteDefError::EndpointMismatch
        | RouteDefError::DegenerateRoute
        | RouteDefError::ZeroThrust
        | RouteDefError::ZeroCapacity
        | RouteDefError::UnknownDriveClass { .. }
        | RouteDefError::Overflow => PlanError::Overflow,
    }
}

/// The drive's coast rate, km/s.
fn coast_kms(law: &DriveLaw) -> u32 {
    match law {
        DriveLaw::Coast { kms, .. } => *kms,
    }
}

/// One leg's laden seconds from its unladen seconds: mush-sim's one laden step
/// under a nominal-thrust profile of the hull's class and rail (§4.4, D-18-44).
fn laden(unladen: u32, drive: &HullDrive, payload_kg: u64) -> Result<u32, PlanError> {
    let profile = DriveProfile::new(drive.class.clone(), 1_000, drive.payload_ceiling_kg)
        .map_err(route_error)?
        .with_law(DistanceLaw::Coast);
    mush_sim::laden_leg_durations(&[unladen], &profile, payload_kg)
        .map_err(route_error)?
        .first()
        .copied()
        .ok_or(PlanError::Overflow)
}

/// **The fixed-point intercept, pass by pass** (§4.4): element `k` is
/// `(dist_km, laden secs)` after pass `k + 1`.
///
/// Before the first pass `t1 := t0`; each pass asks the sky for the chord from
/// `a` at `t0` to `b` at `t1`, takes `ceil(dist / kms)` unladen seconds, lades
/// them through mush-sim's step, and sets `t1 := t0 + laden`. The planner stops
/// at [`INTERCEPT_PASSES`]; a test asks for more to show the contraction.
///
/// # Errors
///
/// [`PlanError::UnknownBody`] when the sky does not answer for `a` or `b`,
/// [`PlanError::Overloaded`] past the rail, [`PlanError::Overflow`] on any
/// arithmetic that leaves its type.
pub fn intercept_passes(
    sky: &Sky,
    a: &CanonicalId,
    b: &CanonicalId,
    t0: i64,
    drive: &HullDrive,
    payload_kg: u64,
    passes: usize,
) -> Result<Vec<(u64, u32)>, PlanError> {
    let kms = u64::from(coast_kms(&drive.law));
    if kms == 0 {
        return Err(PlanError::Overflow);
    }
    let mut out: Vec<(u64, u32)> = Vec::with_capacity(passes);
    let mut t1 = t0;
    for _ in 0..passes {
        let dist = sky
            .distance_km(a, t0, b, t1)
            .ok_or_else(|| PlanError::UnknownBody {
                place: b.to_string(),
            })?;
        let unladen = u32::try_from(dist.div_ceil(kms)).map_err(|_| PlanError::Overflow)?;
        let secs = laden(unladen, drive, payload_kg)?;
        t1 = t0.checked_add(i64::from(secs)).ok_or(PlanError::Overflow)?;
        out.push((dist, secs));
    }
    Ok(out)
}

/// The body a pack place sits on: `NoSkyLeg` for a place that is no location
/// or carries no body, `UnknownBody` for a body the sky does not answer for.
fn body_of(sky: &Sky, pack: &RulesPack, place: &CanonicalId) -> Result<CanonicalId, PlanError> {
    let body = match pack.entry(place) {
        Some(PackEntry::Location(location)) => location.body.clone(),
        _ => None,
    }
    .ok_or_else(|| PlanError::NoSkyLeg {
        place: place.to_string(),
    })?;
    if sky.epoch_of(&body).is_none() {
        return Err(PlanError::UnknownBody {
            place: place.to_string(),
        });
    }
    Ok(body)
}

/// **Plan one lane route at departure** (§4.4, §4.8; D-18-44, D-18-46).
///
/// The route entry by id in `pack`; the hull's drive class must be the route's;
/// the payload must be within the rail. Each leg is walked as a body pair from
/// the route's `from` place: a cross-body leg by [`INTERCEPT_PASSES`] of
/// [`intercept_passes`] (the last pass's figures), a same-body leg by its
/// authored `base_s` through the same laden step (`dist_km` 0). The next leg
/// starts where this one ends. Then the burn ([`passage_burn`]), the crew-days
/// ([`crew_days`]), the law's word and the sky's revision; the plan's own check
/// runs before it is returned.
///
/// # Errors
///
/// [`PlanError`], as its variants say; a plan that fails its own check is
/// [`PlanError::Overflow`].
#[allow(clippy::too_many_arguments)] // §4.8's signature, kept whole (the CarrierJourney::new precedent)
pub fn plan_passage(
    sky: &Sky,
    pack: &RulesPack,
    route: &CanonicalId,
    drive: &HullDrive,
    dry_kg: u64,
    payload_kg: u64,
    occupants: u32,
    depart_ic: i64,
) -> Result<PassagePlan, PlanError> {
    let Some(PackEntry::Route(entry)) = pack.entry(route) else {
        return Err(PlanError::UnknownRoute {
            route: route.to_string(),
        });
    };
    if entry.drive_class != drive.class {
        return Err(PlanError::UnknownDriveClass {
            route: route.to_string(),
            expected: entry.drive_class.to_string(),
            actual: drive.class.to_string(),
        });
    }
    if payload_kg > drive.payload_ceiling_kg {
        return Err(PlanError::Overloaded {
            payload_kg,
            ceiling_kg: drive.payload_ceiling_kg,
        });
    }
    let mut legs: Vec<LegPlan> = Vec::with_capacity(entry.legs.len());
    let mut from = body_of(sky, pack, &entry.from)?;
    let mut t0 = depart_ic;
    for leg in &entry.legs {
        let to = body_of(sky, pack, &leg.to)?;
        let (dist_km, secs) = if to == from {
            (0, laden(leg.base_s, drive, payload_kg)?)
        } else {
            intercept_passes(sky, &from, &to, t0, drive, payload_kg, INTERCEPT_PASSES)?
                .last()
                .copied()
                .ok_or(PlanError::Overflow)?
        };
        let t1 = t0.checked_add(i64::from(secs)).ok_or(PlanError::Overflow)?;
        legs.push(LegPlan {
            to: leg.to.clone(),
            body: to.clone(),
            dist_km,
            secs,
            t0,
            t1,
        });
        from = to;
        t0 = t1;
    }
    let total_s = legs
        .last()
        .map(|last| last.t1)
        .ok_or(PlanError::Overflow)?
        .checked_sub(depart_ic)
        .ok_or(PlanError::Overflow)?;
    let burn_t = passage_burn(&drive.law, dry_kg, payload_kg, &legs)?;
    let crew = crew_days(occupants, &legs)?;
    let plan = PassagePlan {
        route: route.clone(),
        law: CanonicalId::new(drive.law.word()).map_err(|_| PlanError::Overflow)?,
        depart_ic,
        sky_rev: sky.revision().map_err(|_| PlanError::SkyUnavailable)?,
        legs,
        total_s,
        burn_t,
        crew_days: crew,
    };
    plan.check().map_err(|_| PlanError::Overflow)?;
    Ok(plan)
}

/// **Choose a route to a destination** (D-18-47): plan each listed route in
/// order — the first that cannot be planned refuses (a listed route that cannot
/// be planned is a content defect, never skipped; lead ruling L22) — and keep
/// the least `total_s`, ties to the earlier listing. `via` is reserved.
///
/// # Errors
///
/// [`PlanError::ViaReserved`] for any `via`, [`PlanError::UnknownRoute`] for an
/// empty list, and the first route's own refusal.
#[allow(clippy::too_many_arguments)] // §4.8's signature, kept whole
pub fn choose_route(
    sky: &Sky,
    pack: &RulesPack,
    routes: &[CanonicalId],
    drive: &HullDrive,
    dry_kg: u64,
    payload_kg: u64,
    occupants: u32,
    depart_ic: i64,
    via: Option<&CanonicalId>,
) -> Result<PassagePlan, PlanError> {
    if via.is_some() {
        return Err(PlanError::ViaReserved);
    }
    let mut best: Option<PassagePlan> = None;
    for route in routes {
        let plan = plan_passage(
            sky, pack, route, drive, dry_kg, payload_kg, occupants, depart_ic,
        )?;
        let better = best
            .as_ref()
            .is_none_or(|current| plan.total_s < current.total_s);
        if better {
            best = Some(plan);
        }
    }
    best.ok_or_else(|| PlanError::UnknownRoute {
        route: "(none listed)".to_owned(),
    })
}

/// **The drive law's fuel function** (§4.4 "The fuel function is the planner's
/// output"; lead ruling L19): the Coast law burns a fraction of departure mass
/// per departure, distance-free, so it ignores `legs` — `ceil((dry + payload) ×
/// burn_permille / 1 000 000)` **whole tonnes**, in checked `u128` (105‰ of
/// 382,184 kg is 41 t). The flip-and-burn law is post-slice cut P-A's.
///
/// # Errors
///
/// [`PlanError::Overflow`] when the arithmetic leaves its type.
pub fn passage_burn(
    law: &DriveLaw,
    dry_kg: u64,
    payload_kg: u64,
    legs: &[LegPlan],
) -> Result<u64, PlanError> {
    let _ = legs;
    match law {
        DriveLaw::Coast { burn_permille, .. } => {
            let mass = u128::from(dry_kg)
                .checked_add(u128::from(payload_kg))
                .ok_or(PlanError::Overflow)?;
            let grams = mass
                .checked_mul(u128::from(*burn_permille))
                .ok_or(PlanError::Overflow)?;
            u64::try_from(grams.div_ceil(1_000_000)).map_err(|_| PlanError::Overflow)
        }
    }
}

/// **Crew-days** (D-18-30, the one rule): `occupants × Σ ceil(secs / 86 400)`,
/// each leg's whole IC days, checked.
///
/// # Errors
///
/// [`PlanError::Overflow`] when the arithmetic leaves `u32`.
pub fn crew_days(occupants: u32, legs: &[LegPlan]) -> Result<u32, PlanError> {
    let mut days: u32 = 0;
    for leg in legs {
        days = days
            .checked_add(leg.secs.div_ceil(DAY_SECS))
            .ok_or(PlanError::Overflow)?;
    }
    occupants.checked_mul(days).ok_or(PlanError::Overflow)
}

/// **The payload** (§4.2/§4.4; lead ruling L12): Σ over `accepted` goods of
/// `read(good) × mass_kg_per_unit`, plus `occupants × allowance_kg`, checked.
/// One function for `+ship/plan`, `+ship/undock` and the burn's prepare, so the
/// three can never disagree on what the hull weighs.
///
/// # Errors
///
/// [`PlanError::Unreadable`] for a good the table does not weigh, `read`'s own
/// refusal, and [`PlanError::Overflow`].
pub fn payload_kg(
    goods: &GoodsTable,
    accepted: &[CanonicalId],
    mut read: impl FnMut(&CanonicalId) -> Result<u64, PlanError>,
    occupants: u32,
    allowance_kg: u32,
) -> Result<u64, PlanError> {
    let mut total = u64::from(occupants)
        .checked_mul(u64::from(allowance_kg))
        .ok_or(PlanError::Overflow)?;
    for good in accepted {
        let per_unit = goods
            .good(good)
            .and_then(crate::definitions::GoodDef::mass_kg_per_unit)
            .ok_or_else(|| PlanError::Unreadable {
                what: format!("the mass of {good}"),
            })?;
        let qty = read(good)?;
        total = qty
            .checked_mul(per_unit)
            .and_then(|kg| total.checked_add(kg))
            .ok_or(PlanError::Overflow)?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leg(secs: u32) -> LegPlan {
        LegPlan {
            to: CanonicalId::new("x").unwrap(),
            body: CanonicalId::new("x").unwrap(),
            dist_km: 0,
            secs,
            t0: 0,
            t1: i64::from(secs),
        }
    }

    #[test]
    fn ic9_the_coast_burn_is_whole_tonnes_of_departure_mass() {
        let law = DriveLaw::Coast {
            kms: 226,
            burn_permille: 105,
        };
        assert_eq!(passage_burn(&law, 240_000, 142_184, &[]), Ok(41));
        assert_eq!(passage_burn(&law, 240_000, 106_176, &[]), Ok(37));
        // L27: the full cargo with 40 t aboard burns exactly 40 t; with 39 t, 40 t.
        assert_eq!(passage_burn(&law, 240_000, 140_184, &[]), Ok(40));
        assert_eq!(passage_burn(&law, 240_000, 139_184, &[]), Ok(40));
        assert_eq!(passage_burn(&law, u64::MAX, u64::MAX, &[]), {
            let grams = (u128::from(u64::MAX) * 2) * 105;
            u64::try_from(grams.div_ceil(1_000_000)).map_err(|_| PlanError::Overflow)
        });
    }

    #[test]
    fn ic9_crew_days_are_whole_days_per_leg_times_occupants() {
        assert_eq!(crew_days(1, &[leg(959_361), leg(735_600)]), Ok(21));
        assert_eq!(crew_days(1, &[leg(653_642), leg(852_473)]), Ok(18));
        assert_eq!(crew_days(2, &[leg(86_400), leg(1)]), Ok(4));
        assert_eq!(
            crew_days(u32::MAX, &[leg(86_401)]),
            Err(PlanError::Overflow)
        );
    }
}
