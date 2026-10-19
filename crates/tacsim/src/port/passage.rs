//! **The passage planner** (doc 18 cut IC9; doc 18 §4.4 "Passage: the planner
//! over the sky at departure", "The fuel function is the planner's output",
//! §4.8 "The seams the slice builds"; D-18-11, D-18-30, D-18-44 … D-18-47; doc 19
//! cut A.2 §4, D-19-8 … D-19-13).
//!
//! Pure: no `World`, no `Ctx`, checked integer arithmetic throughout except the
//! laws' own physics (D-19-8: the brachistochrone's square root, the rocket
//! equation's exponential and the keep-out's arc, each in `f64` and rounded once
//! to a whole second, tonne or kilometre; a committed passage is frozen, T1, so a
//! platform's last-place difference is harmless). One planner, asked once, at
//! departure:
//!
//! - [`plan_passage`] walks a lane route's legs as **body pairs** and, per leg,
//!   finds the intercept by a fixed number of passes ([`intercept_passes`],
//!   [`INTERCEPT_PASSES`]): the chord from the origin body at the leg's start to
//!   the target body at the leg's end — or, when that chord passes inside the
//!   solar keep-out, the shortest path around it ([`leg_distance_km`],
//!   [`KEEP_OUT_AU`]; a cost, never a closure, D-19-12) — the law's seconds for
//!   that distance at the hull's load ([`leg_seconds`]), and the end instant fed
//!   back. A leg between two places on one body uses its authored `base_s`
//!   through mush-sim's laden step under either law.
//! - [`leg_seconds`] is the law (D-19-8): **Coast** takes `ceil(dist / kms)`
//!   unladen seconds and lades them through mush-sim's one step
//!   ([`mush_sim::laden_leg_durations`], `route_duration`'s floor arithmetic,
//!   lead ruling L1); **Flip** is the brachistochrone `t = 2√(d / a)` at the
//!   hull's acceleration for its load ([`flip_accel_mps2`]: thrust-limited, so
//!   `accel_mg` is the acceleration at the rated maximum departure mass and a
//!   lighter hull is faster) scaled by the passage's throttle
//!   ([`HullDrive::throttle_permille`], D-19-9).
//! - [`passage_burn`] is the law's fuel function: Coast a fraction of departure
//!   mass, distance-free (§4.4, lead ruling L19); Flip the rocket equation over
//!   the passage's whole `Δv = Σ a·t`, so fuel moves with the sky (D-19-8).
//!   Both in whole tonnes, rounded up. [`crew_days`] is D-18-30's one rule;
//!   [`payload_kg`] is the payload the planner, `+ship/plan`, `+ship/undock` and
//!   the burn's prepare all sum the same way (lead ruling L12).
//! - [`choose_route`] plans every route a port lists for a destination, keeps
//!   the best under a [`RoutePolicy`] — the fastest (`total_s`, the queue's) or
//!   the cheapest (`burn_t`), ties to the earlier listing (D-18-47, D-19-13) —
//!   and honours `via`, a waypoint place the route must call at; a listed route
//!   that cannot be planned refuses (lead ruling L22).
//!
//! The plan is frozen in the burn's ticket (D-18-46): nothing after the burn
//! re-reads the sky, and the journey flies the plan's own seconds. **No slice
//! indexing** anywhere in this file (tacsim's restriction census, reviewer F4):
//! legs are walked with iterators, `.first()` and `.last()`.

use mush_core::CanonicalId;
use mush_sim::{DistanceLaw, DriveProfile, RouteDefError};

use crate::definitions::GoodsTable;
use crate::ephemeris::AU_KM;
use crate::pack::{PackEntry, RulesPack};
use crate::sky::{PosKm, Sky};

pub use crate::definitions::logistics::DriveLaw;
pub use crate::port::families::{LegPlan, PassagePlan};

/// How many fixed-point passes [`plan_passage`] makes per leg (§4.4; five since
/// doc 19 cut A.2, D-19-11: a flip leg's seconds grow with the square root of
/// its distance, a weaker contraction than coast's).
pub const INTERCEPT_PASSES: usize = 5;

/// Seconds in one IC day: the crew-day rule's divisor (D-18-30).
pub const DAY_SECS: u32 = 86_400;

/// Standard gravity, metres per second squared: what a milli-g is a thousandth of.
pub const G0_MPS2: f64 = 9.806_65;

/// **The solar keep-out** (doc 19 D-19-12; doc 18 Q-18-40): a leg whose chord
/// passes inside this radius of the Sun is planned along the shortest path
/// around it. Mercury's orbit is 0.39 AU.
pub const KEEP_OUT_AU: f64 = 0.3;

/// The slowest a passage may be flown, permille of the hull's thrust (D-19-9,
/// Q-19-4: at most twice its full-thrust length).
pub const MIN_THROTTLE_PERMILLE: u32 = 250;
/// Full thrust.
pub const MAX_THROTTLE_PERMILLE: u32 = 1_000;
/// The throttle a passage is flown at unless the captain says otherwise.
pub const DEFAULT_THROTTLE_PERMILLE: u32 = MAX_THROTTLE_PERMILLE;

/// **A hull's drive, as the planner reads it** (§4.8): its class, the law the
/// class flies under, the payload rail (`route_duration`'s laden denominator and
/// the flip law's rated maximum load) and, since doc 19 cut A.2, the throttle
/// this passage is flown at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HullDrive {
    /// The drive class a route's `drive_class` names (`coast_hauler`).
    pub class: CanonicalId,
    /// The law (`flip {accel_mg 50, exhaust_kms 6000}` for the Dray since A.2;
    /// `coast {kms 226, burn_permille 105}` in the slice).
    pub law: DriveLaw,
    /// The payload rail, kilograms.
    pub payload_ceiling_kg: u64,
    /// The throttle, permille of the hull's thrust;
    /// `MIN_THROTTLE_PERMILLE..=MAX_THROTTLE_PERMILLE`. Read by the flip law
    /// only (a coast hull cruises at its one speed).
    pub throttle_permille: u32,
}

impl HullDrive {
    /// The same drive at `permille` of its thrust.
    #[must_use]
    pub fn at_throttle(mut self, permille: u32) -> HullDrive {
        self.throttle_permille = permille;
        self
    }
}

/// **Which listed route `choose_route` keeps** (D-19-13).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RoutePolicy {
    /// The least `total_s`; the queue's and the default.
    #[default]
    Fastest,
    /// The least `burn_t`, ties to the faster.
    Cheapest,
}

impl RoutePolicy {
    /// The policy's word (`fastest`, `cheapest`).
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            RoutePolicy::Fastest => "fastest",
            RoutePolicy::Cheapest => "cheapest",
        }
    }

    /// The word's policy.
    #[must_use]
    pub fn parse(word: &str) -> Option<RoutePolicy> {
        match word.trim().to_ascii_lowercase().as_str() {
            "fastest" | "fast" => Some(RoutePolicy::Fastest),
            "cheapest" | "cheap" | "thrifty" => Some(RoutePolicy::Cheapest),
            _ => None,
        }
    }
}

/// **Why a passage could not be planned** (§4.8's six, plus `SkyUnavailable`,
/// lead ruling L10, plus `Unreadable` — the payload's own read; doc 19 cut A.2
/// adds `Throttle` and `NoRouteVia` and retires `ViaReserved`).
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
    /// The throttle is outside its band.
    #[error(
        "a passage is flown at {MIN_THROTTLE_PERMILLE} to {MAX_THROTTLE_PERMILLE} permille of thrust; {permille} is not"
    )]
    Throttle {
        /// The throttle asked for.
        permille: u32,
    },
    /// No listed route calls at the waypoint asked for.
    #[error("no listed route to that port calls at {via}")]
    NoRouteVia {
        /// The waypoint.
        via: String,
    },
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

/// One leg's laden seconds from its unladen seconds: mush-sim's one laden step
/// under a nominal-thrust profile of the hull's class and rail (§4.4, D-18-44).
/// The Coast law's every leg and either law's same-body legs.
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

/// The throttle, checked.
fn throttle(drive: &HullDrive) -> Result<u32, PlanError> {
    let permille = drive.throttle_permille;
    if !(MIN_THROTTLE_PERMILLE..=MAX_THROTTLE_PERMILLE).contains(&permille) {
        return Err(PlanError::Throttle { permille });
    }
    Ok(permille)
}

/// **The flip law's acceleration for a load**, metres per second squared
/// (D-19-8): `accel_mg` is the acceleration at the rated maximum departure mass
/// (`dry + payload_ceiling`), the drive is thrust-limited, so at `dry + payload`
/// it is `accel_mg · g₀ · (dry + ceiling) / (dry + payload)`, times the throttle.
/// `None` for a coast drive or an impossible mass.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn flip_accel_mps2(drive: &HullDrive, dry_kg: u64, payload_kg: u64) -> Option<f64> {
    let DriveLaw::Flip { accel_mg, .. } = drive.law else {
        return None;
    };
    let rated = dry_kg.checked_add(drive.payload_ceiling_kg)? as f64;
    let mass = dry_kg.checked_add(payload_kg)? as f64;
    if mass <= 0.0 || rated <= 0.0 {
        return None;
    }
    let full = f64::from(accel_mg) / 1_000.0 * G0_MPS2 * rated / mass;
    Some(full * f64::from(drive.throttle_permille) / 1_000.0)
}

/// **One leg's seconds under the law** (D-19-8): Coast lades `ceil(dist / kms)`
/// through mush-sim's step; Flip is the brachistochrone `ceil(2√(d / a))` at the
/// hull's acceleration for its load and throttle. A zero distance is a zero leg
/// (the caller uses the authored `base_s` for a same-body leg).
///
/// # Errors
///
/// [`PlanError::Overloaded`] past the rail, [`PlanError::Throttle`] outside the
/// band, [`PlanError::Overflow`] on any arithmetic that leaves its type.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn leg_seconds(
    drive: &HullDrive,
    dry_kg: u64,
    payload_kg: u64,
    dist_km: u64,
) -> Result<u32, PlanError> {
    if payload_kg > drive.payload_ceiling_kg {
        return Err(PlanError::Overloaded {
            payload_kg,
            ceiling_kg: drive.payload_ceiling_kg,
        });
    }
    match drive.law {
        DriveLaw::Coast { kms, .. } => {
            let kms = u64::from(kms);
            if kms == 0 {
                return Err(PlanError::Overflow);
            }
            let unladen = u32::try_from(dist_km.div_ceil(kms)).map_err(|_| PlanError::Overflow)?;
            laden(unladen, drive, payload_kg)
        }
        DriveLaw::Flip { .. } => {
            throttle(drive)?;
            let a = flip_accel_mps2(drive, dry_kg, payload_kg).ok_or(PlanError::Overflow)?;
            if a <= 0.0 || !a.is_finite() {
                return Err(PlanError::Overflow);
            }
            #[allow(clippy::cast_precision_loss)]
            let d_m = dist_km as f64 * 1_000.0;
            let secs = (2.0 * (d_m / a).sqrt()).ceil();
            if !secs.is_finite() || secs < 0.0 || secs > f64::from(u32::MAX) {
                return Err(PlanError::Overflow);
            }
            Ok(secs as u32)
        }
    }
}

/// **The shortest path around the solar keep-out** (D-19-12), kilometres,
/// rounded up: the chord from `a` to `b` when it clears a disc of `radius_km`
/// about the Sun (or when either end is inside it — a port that close plans its
/// chord); else tangent, arc, tangent: `√(|A|² − R²) + √(|B|² − R²) + R·θ`,
/// with `θ` the angle between the two tangent points, in the plane of the two
/// positions.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn detour_km(a: PosKm, b: PosKm, radius_km: f64) -> u64 {
    let chord = a.distance_km(b);
    let (ax, ay, az) = (a.x as f64, a.y as f64, a.z as f64);
    let (bx, by, bz) = (b.x as f64, b.y as f64, b.z as f64);
    let (dx, dy, dz) = (bx - ax, by - ay, bz - az);
    let len2 = dx * dx + dy * dy + dz * dz;
    if len2 <= 0.0 {
        return chord;
    }
    // The segment's closest approach to the Sun, strictly between its ends.
    let t = -(ax * dx + ay * dy + az * dz) / len2;
    if t <= 0.0 || t >= 1.0 {
        return chord;
    }
    let (px, py, pz) = (ax + t * dx, ay + t * dy, az + t * dz);
    let closest = (px * px + py * py + pz * pz).sqrt();
    if closest >= radius_km {
        return chord;
    }
    let ra = (ax * ax + ay * ay + az * az).sqrt();
    let rb = (bx * bx + by * by + bz * bz).sqrt();
    if ra <= radius_km || rb <= radius_km {
        return chord;
    }
    let cos_aob = ((ax * bx + ay * by + az * bz) / (ra * rb)).clamp(-1.0, 1.0);
    let theta = (cos_aob.acos() - (radius_km / ra).acos() - (radius_km / rb).acos()).max(0.0);
    let path = (ra * ra - radius_km * radius_km).sqrt()
        + (rb * rb - radius_km * radius_km).sqrt()
        + radius_km * theta;
    let path = path.ceil();
    if !path.is_finite() || path < chord as f64 {
        return chord;
    }
    path as u64
}

/// **One leg's distance** (D-19-12): the chord from `a` at `ta` to `b` at `tb`
/// through the one sky query, or the keep-out detour when the chord passes
/// inside [`KEEP_OUT_AU`]. `None` for a body the sky does not know.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn leg_distance_km(
    sky: &Sky,
    a: &CanonicalId,
    ta: i64,
    b: &CanonicalId,
    tb: i64,
) -> Option<u64> {
    let chord = sky.distance_km(a, ta, b, tb)?;
    let (pa, pb) = (sky.position_km(a, ta)?, sky.position_km(b, tb)?);
    let detour = detour_km(pa, pb, KEEP_OUT_AU * AU_KM as f64);
    Some(detour.max(chord))
}

/// **The fixed-point intercept, pass by pass** (§4.4): element `k` is
/// `(dist_km, secs)` after pass `k + 1`.
///
/// Before the first pass `t1 := t0`; each pass asks the sky for the leg's
/// distance from `a` at `t0` to `b` at `t1` ([`leg_distance_km`]), takes the
/// law's seconds for it ([`leg_seconds`]) and sets `t1 := t0 + secs`. The
/// planner stops at [`INTERCEPT_PASSES`]; a test asks for more to show the
/// contraction.
///
/// # Errors
///
/// [`PlanError::UnknownBody`] when the sky does not answer for `a` or `b`,
/// [`PlanError::Overloaded`] past the rail, [`PlanError::Throttle`] outside the
/// band, [`PlanError::Overflow`] on any arithmetic that leaves its type.
#[allow(clippy::too_many_arguments)] // the planner's own inputs, kept whole
pub fn intercept_passes(
    sky: &Sky,
    a: &CanonicalId,
    b: &CanonicalId,
    t0: i64,
    drive: &HullDrive,
    dry_kg: u64,
    payload_kg: u64,
    passes: usize,
) -> Result<Vec<(u64, u32)>, PlanError> {
    let mut out: Vec<(u64, u32)> = Vec::with_capacity(passes);
    let mut t1 = t0;
    for _ in 0..passes {
        let dist = leg_distance_km(sky, a, t0, b, t1).ok_or_else(|| PlanError::UnknownBody {
            place: b.to_string(),
        })?;
        let secs = leg_seconds(drive, dry_kg, payload_kg, dist)?;
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

/// **Plan one lane route at departure** (§4.4, §4.8; D-18-44, D-18-46; D-19-8).
///
/// The route entry by id in `pack`; the hull's drive class must be the route's;
/// the payload must be within the rail and the throttle within its band. Each
/// leg is walked as a body pair from the route's `from` place: a cross-body leg
/// by [`INTERCEPT_PASSES`] of [`intercept_passes`] (the last pass's figures), a
/// same-body leg by its authored `base_s` through mush-sim's laden step
/// (`dist_km` 0). The next leg starts where this one ends. Then the burn
/// ([`passage_burn`]), the crew-days ([`crew_days`]), the law's word and the
/// sky's revision; the plan's own check runs before it is returned.
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
    if matches!(drive.law, DriveLaw::Flip { .. }) {
        throttle(drive)?;
    }
    let mut legs: Vec<LegPlan> = Vec::with_capacity(entry.legs.len());
    let mut from = body_of(sky, pack, &entry.from)?;
    let mut t0 = depart_ic;
    for leg in &entry.legs {
        let to = body_of(sky, pack, &leg.to)?;
        let (dist_km, secs) = if to == from {
            (0, laden(leg.base_s, drive, payload_kg)?)
        } else {
            intercept_passes(
                sky,
                &from,
                &to,
                t0,
                drive,
                dry_kg,
                payload_kg,
                INTERCEPT_PASSES,
            )?
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
    let burn_t = passage_burn(drive, dry_kg, payload_kg, &legs)?;
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
        throttle_permille: drive.throttle_permille,
    };
    plan.check().map_err(|_| PlanError::Overflow)?;
    Ok(plan)
}

/// Whether a listed route calls at `via` before its destination.
fn calls_at(pack: &RulesPack, route: &CanonicalId, via: &CanonicalId) -> bool {
    let Some(PackEntry::Route(entry)) = pack.entry(route) else {
        return false;
    };
    let waypoints = entry.legs.len().saturating_sub(1);
    entry.legs.iter().take(waypoints).any(|leg| &leg.to == via)
}

/// **Choose a route to a destination** (D-18-47; D-19-13): plan each listed
/// route in order — the first that cannot be planned refuses (a listed route
/// that cannot be planned is a content defect, never skipped; lead ruling L22) —
/// and keep the best under `policy`, ties to the earlier listing. With `via`,
/// only the listed routes that call at that place before their destination are
/// considered.
///
/// # Errors
///
/// [`PlanError::NoRouteVia`] when no listed route calls at `via`,
/// [`PlanError::UnknownRoute`] for an empty list, and the first route's own
/// refusal.
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
    policy: RoutePolicy,
    via: Option<&CanonicalId>,
) -> Result<PassagePlan, PlanError> {
    let considered: Vec<&CanonicalId> = match via {
        Some(via) => {
            let calling: Vec<&CanonicalId> = routes
                .iter()
                .filter(|route| calls_at(pack, route, via))
                .collect();
            if calling.is_empty() {
                return Err(PlanError::NoRouteVia {
                    via: via.to_string(),
                });
            }
            calling
        }
        None => routes.iter().collect(),
    };
    let mut best: Option<PassagePlan> = None;
    for route in considered {
        let plan = plan_passage(
            sky, pack, route, drive, dry_kg, payload_kg, occupants, depart_ic,
        )?;
        let better = best.as_ref().is_none_or(|current| match policy {
            RoutePolicy::Fastest => plan.total_s < current.total_s,
            RoutePolicy::Cheapest => {
                plan.burn_t < current.burn_t
                    || (plan.burn_t == current.burn_t && plan.total_s < current.total_s)
            }
        });
        if better {
            best = Some(plan);
        }
    }
    best.ok_or_else(|| PlanError::UnknownRoute {
        route: "(none listed)".to_owned(),
    })
}

/// **The flip law's figures for a planned passage** (D-19-8), for `+ship/plan`
/// and the chart: the acceleration flown (m/s²), the whole passage's `Δv` and
/// the peak velocity of its longest leg (both m/s). `None` under the coast law.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlipProfile {
    /// The acceleration at this load and throttle, m/s².
    pub accel_mps2: f64,
    /// `Σ a·t` over the legs, m/s.
    pub dv_mps: f64,
    /// `√(d·a)` of the longest leg, m/s.
    pub peak_mps: f64,
}

/// [`FlipProfile`] of `legs` under `drive` at a load.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn flip_profile(
    drive: &HullDrive,
    dry_kg: u64,
    payload_kg: u64,
    legs: &[LegPlan],
) -> Option<FlipProfile> {
    let a = flip_accel_mps2(drive, dry_kg, payload_kg)?;
    let mut dv = 0.0_f64;
    let mut peak = 0.0_f64;
    for leg in legs {
        if leg.dist_km == 0 {
            continue;
        }
        dv += a * f64::from(leg.secs);
        peak = peak.max((leg.dist_km as f64 * 1_000.0 * a).sqrt());
    }
    Some(FlipProfile {
        accel_mps2: a,
        dv_mps: dv,
        peak_mps: peak,
    })
}

/// **The drive law's fuel function** (§4.4 "The fuel function is the planner's
/// output"; lead ruling L19; D-19-8): Coast burns a fraction of departure mass
/// per departure, distance-free, so it ignores `legs` — `ceil((dry + payload) ×
/// burn_permille / 1 000 000)` **whole tonnes**, in checked `u128` (105‰ of
/// 382,184 kg is 41 t); Flip burns the rocket equation over the passage,
/// `ceil((dry + payload) · (1 − e^(−Δv / vₑ)))` whole tonnes, with `Δv = Σ a·t`
/// over the legs it flew.
///
/// # Errors
///
/// [`PlanError::Overflow`] when the arithmetic leaves its type.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn passage_burn(
    drive: &HullDrive,
    dry_kg: u64,
    payload_kg: u64,
    legs: &[LegPlan],
) -> Result<u64, PlanError> {
    match drive.law {
        DriveLaw::Coast { burn_permille, .. } => {
            let mass = u128::from(dry_kg)
                .checked_add(u128::from(payload_kg))
                .ok_or(PlanError::Overflow)?;
            let grams = mass
                .checked_mul(u128::from(burn_permille))
                .ok_or(PlanError::Overflow)?;
            u64::try_from(grams.div_ceil(1_000_000)).map_err(|_| PlanError::Overflow)
        }
        DriveLaw::Flip { exhaust_kms, .. } => {
            let profile =
                flip_profile(drive, dry_kg, payload_kg, legs).ok_or(PlanError::Overflow)?;
            let exhaust_mps = f64::from(exhaust_kms) * 1_000.0;
            if exhaust_mps <= 0.0 {
                return Err(PlanError::Overflow);
            }
            let mass = dry_kg.checked_add(payload_kg).ok_or(PlanError::Overflow)? as f64;
            let fraction = 1.0 - (-profile.dv_mps / exhaust_mps).exp();
            let tonnes = (mass * fraction / 1_000.0).ceil();
            if !tonnes.is_finite() || tonnes < 0.0 || tonnes > u64::MAX as f64 {
                return Err(PlanError::Overflow);
            }
            Ok(tonnes as u64)
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

    fn cid(s: &str) -> CanonicalId {
        CanonicalId::new(s).expect("a canonical id")
    }

    fn leg(secs: u32) -> LegPlan {
        LegPlan {
            to: cid("x"),
            body: cid("x"),
            dist_km: 0,
            secs,
            t0: 0,
            t1: i64::from(secs),
        }
    }

    fn coast() -> HullDrive {
        HullDrive {
            class: cid("coast_hauler"),
            law: DriveLaw::Coast {
                kms: 226,
                burn_permille: 105,
            },
            payload_ceiling_kg: 181_000,
            throttle_permille: DEFAULT_THROTTLE_PERMILLE,
        }
    }

    /// The Dray's flip law since doc 19 cut A.2: a twentieth of a gravity at
    /// 421 t, exhaust at 2 % of c.
    fn flip() -> HullDrive {
        HullDrive {
            class: cid("coast_hauler"),
            law: DriveLaw::Flip {
                accel_mg: 50,
                exhaust_kms: 6_000,
            },
            payload_ceiling_kg: 181_000,
            throttle_permille: DEFAULT_THROTTLE_PERMILLE,
        }
    }

    #[test]
    fn ic9_the_coast_burn_is_whole_tonnes_of_departure_mass() {
        let drive = coast();
        assert_eq!(passage_burn(&drive, 240_000, 142_184, &[]), Ok(41));
        assert_eq!(passage_burn(&drive, 240_000, 106_176, &[]), Ok(37));
        // L27: the full cargo with 40 t aboard burns exactly 40 t; with 39 t, 40 t.
        assert_eq!(passage_burn(&drive, 240_000, 140_184, &[]), Ok(40));
        assert_eq!(passage_burn(&drive, 240_000, 139_184, &[]), Ok(40));
        assert_eq!(passage_burn(&drive, u64::MAX, u64::MAX, &[]), {
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

    #[test]
    fn a2_the_flip_acceleration_is_thrust_limited_and_throttled() {
        let drive = flip();
        // At the rated maximum mass the acceleration is the authored 50 mg.
        let rated = flip_accel_mps2(&drive, 240_000, 181_000).expect("a flip drive");
        assert!((rated - 0.050 * G0_MPS2).abs() < 1e-9, "{rated}");
        // Lighter is faster: an empty hull (240 t) at 421/240 of it.
        let empty = flip_accel_mps2(&drive, 240_000, 0).expect("a flip drive");
        assert!((empty - rated * 421.0 / 240.0).abs() < 1e-9, "{empty}");
        // Half throttle halves it.
        let half = flip_accel_mps2(&drive.clone().at_throttle(500), 240_000, 181_000)
            .expect("a flip drive");
        assert!((half - rated / 2.0).abs() < 1e-9, "{half}");
        assert_eq!(flip_accel_mps2(&coast(), 240_000, 0), None);
    }

    #[test]
    fn a2_the_flip_leg_is_the_brachistochrone() {
        let drive = flip();
        // 1 AU at the rated load: t = 2 sqrt(d / a).
        let d_km = AU_KM;
        let a = 0.050 * G0_MPS2;
        #[allow(clippy::cast_precision_loss)]
        let want = (2.0 * ((d_km as f64 * 1_000.0) / a).sqrt()).ceil();
        let secs = leg_seconds(&drive, 240_000, 181_000, d_km).expect("plans");
        #[allow(clippy::cast_precision_loss)]
        let got = f64::from(secs);
        assert!((got - want).abs() < 1.0, "{got} against {want}");
        // Four times the distance is twice the time; half the throttle is sqrt 2 longer.
        let far = leg_seconds(&drive, 240_000, 181_000, d_km * 4).expect("plans");
        assert!((f64::from(far) - 2.0 * got).abs() <= 2.0);
        let slow =
            leg_seconds(&drive.clone().at_throttle(500), 240_000, 181_000, d_km).expect("plans");
        assert!((f64::from(slow) - got * 2.0_f64.sqrt()).abs() <= 2.0);
        // The band.
        assert_eq!(
            leg_seconds(&drive.clone().at_throttle(100), 240_000, 0, d_km),
            Err(PlanError::Throttle { permille: 100 })
        );
        assert_eq!(
            leg_seconds(&drive, 240_000, 181_001, d_km),
            Err(PlanError::Overloaded {
                payload_kg: 181_001,
                ceiling_kg: 181_000
            })
        );
        // The coast law is unchanged: ceil(d / kms), laden.
        assert_eq!(leg_seconds(&coast(), 240_000, 0, 226_000), Ok(1_000));
    }

    #[test]
    fn a2_the_flip_burn_is_the_rocket_equation_and_grows_with_distance() {
        let drive = flip();
        let plan = |d_km: u64| -> (u32, u64) {
            let secs = leg_seconds(&drive, 240_000, 142_188, d_km).expect("plans");
            let legs = [LegPlan {
                to: cid("x"),
                body: cid("x"),
                dist_km: d_km,
                secs,
                t0: 0,
                t1: i64::from(secs),
            }];
            (
                secs,
                passage_burn(&drive, 240_000, 142_188, &legs).expect("burns"),
            )
        };
        let (near_s, near_t) = plan(AU_KM);
        let (far_s, far_t) = plan(AU_KM * 4);
        assert!(
            far_s > near_s && far_t > near_t,
            "farther is longer and costlier"
        );
        // Δv = a·t; the fraction is 1 − e^(−Δv/vₑ); whole tonnes up.
        let a = flip_accel_mps2(&drive, 240_000, 142_188).expect("a");
        let dv = a * f64::from(near_s);
        #[allow(clippy::cast_precision_loss)]
        let want = ((240_000.0 + 142_188.0) * (1.0 - (-dv / 6_000_000.0).exp()) / 1_000.0).ceil();
        #[allow(clippy::cast_precision_loss)]
        let got = near_t as f64;
        assert!((got - want).abs() < 0.5, "{got} against {want}");
        // A half-throttle passage burns less (Δv ∝ √a) and takes longer.
        let slow = drive.clone().at_throttle(500);
        let secs = leg_seconds(&slow, 240_000, 142_188, AU_KM).expect("plans");
        let legs = [LegPlan {
            to: cid("x"),
            body: cid("x"),
            dist_km: AU_KM,
            secs,
            t0: 0,
            t1: i64::from(secs),
        }];
        let thrifty = passage_burn(&slow, 240_000, 142_188, &legs).expect("burns");
        assert!(thrifty < near_t && secs > near_s, "{thrifty} t in {secs} s");
        // No legs flown, nothing burned.
        assert_eq!(passage_burn(&drive, 240_000, 142_188, &[]), Ok(0));
    }

    #[test]
    fn a2_the_keep_out_detours_a_chord_through_the_sun_and_leaves_others_alone() {
        let au = AU_KM as f64;
        let r = KEEP_OUT_AU * au;
        let at = |x: f64, y: f64| PosKm {
            #[allow(clippy::cast_possible_truncation)]
            x: (x * au).round() as i64,
            #[allow(clippy::cast_possible_truncation)]
            y: (y * au).round() as i64,
            z: 0,
        };
        // Opposite sides of the Sun: the chord runs through it; the detour is the
        // two tangents plus the arc, longer than the chord by about 2R(π/2 − 1)...
        let (a, b) = (at(1.0, 0.0), at(-1.0, 0.0));
        let chord = a.distance_km(b);
        let detour = detour_km(a, b, r);
        assert!(detour > chord, "{detour} over {chord}");
        // ...exactly: 2·sqrt(1 − R²) + R·(π − 2·acos(R)).
        let want = 2.0 * (1.0 - KEEP_OUT_AU * KEEP_OUT_AU).sqrt()
            + KEEP_OUT_AU * (std::f64::consts::PI - 2.0 * KEEP_OUT_AU.acos());
        #[allow(clippy::cast_precision_loss)]
        let got = detour as f64 / au;
        assert!((got - want).abs() < 1e-6, "{got} against {want}");
        // A chord that clears the disc is the chord.
        let (c, d) = (at(1.0, 0.0), at(0.0, 1.0));
        assert_eq!(detour_km(c, d, r), c.distance_km(d));
        // Ceres to Mars at the lane's epoch clears it by a wide margin.
        let (ceres, mars) = (at(2.866, 0.0), at(1.4454, 0.1));
        assert_eq!(detour_km(ceres, mars, r), ceres.distance_km(mars));
        // An end inside the disc plans its chord.
        let (inside, out) = (at(0.1, 0.0), at(-1.0, 0.0));
        assert_eq!(detour_km(inside, out, r), inside.distance_km(out));
    }

    #[test]
    fn a2_the_route_policy_words_round_trip() {
        for policy in [RoutePolicy::Fastest, RoutePolicy::Cheapest] {
            assert_eq!(RoutePolicy::parse(policy.word()), Some(policy));
        }
        assert_eq!(RoutePolicy::parse("thrifty"), Some(RoutePolicy::Cheapest));
        assert_eq!(RoutePolicy::parse("slow"), None);
        assert_eq!(RoutePolicy::default(), RoutePolicy::Fastest);
    }
}
