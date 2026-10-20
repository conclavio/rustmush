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
use serde::{Deserialize, Serialize};

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
    /// **Doc 20 cut SM.2 (D-20-3)** — the hull cannot land at (or lift from) a
    /// site at this load and throttle: its thrust-to-weight on the site's body is
    /// under [`LANDING_TWR_PERMILLE`]. `body` is the body's id (for logs; a
    /// player sentence prints its display name).
    #[error(
        "{place} is on {body}'s surface: thrust-to-weight there is {twr_permille} permille at this load and throttle, under the 1200 a landing needs"
    )]
    Landing {
        /// The site place.
        place: String,
        /// The site's body, by id.
        body: String,
        /// The thrust-to-weight there, permille (0 for a drive with no thrust figure).
        twr_permille: u64,
    },
    /// A quantity the payload sums could not be read: a pool, or a good the
    /// goods table does not weigh.
    #[error("{what} could not be read")]
    Unreadable {
        /// What.
        what: String,
    },
}

/// **The least thrust-to-weight, permille, a leg to or from a site may fly**
/// (doc 20 cut SM.2, D-20-3): a hull that cannot lift 1.2 times its weight on a
/// body cannot land there, so no Dray lands on Luna, Mars or Titan.
pub const LANDING_TWR_PERMILLE: u64 = 1_200;

/// **The least a leg touching a site burns, tonnes** (doc 20 §3.3: a hop is
/// never free; the ascent's few kilograms round up to a tonne).
pub const SITE_MIN_BURN_T: u64 = 1;

/// **The hull's thrust-to-weight on a body of `surface_mg`**, permille, at
/// departure mass and the drive's throttle (doc 20 cut SM.2, D-20-3):
/// `⌊accel_mg × (dry + ceiling) × throttle / ((dry + payload) × surface_mg)⌋`, in
/// checked `u128` — the flip law's acceleration for the load ([`flip_accel_mps2`])
/// over the body's gravity, both in milli-g. `None` for a coast drive (no thrust
/// figure) or a zero divisor.
#[must_use]
pub fn landing_twr_permille(
    drive: &HullDrive,
    dry_kg: u64,
    payload_kg: u64,
    surface_mg: u32,
) -> Option<u64> {
    let DriveLaw::Flip { accel_mg, .. } = drive.law else {
        return None;
    };
    let rated = u128::from(dry_kg).checked_add(u128::from(drive.payload_ceiling_kg))?;
    let thrust = u128::from(accel_mg)
        .checked_mul(rated)?
        .checked_mul(u128::from(drive.throttle_permille))?;
    let weight = u128::from(dry_kg)
        .checked_add(u128::from(payload_kg))?
        .checked_mul(u128::from(surface_mg))?;
    if weight == 0 {
        return None;
    }
    u64::try_from(thrust / weight).ok()
}

/// **The least throttle, permille, at which [`landing_twr_permille`] reaches
/// [`LANDING_TWR_PERMILLE`]** (doc 20 cut SM.2): `max(MIN_THROTTLE_PERMILLE,
/// ⌈1200 × (dry + payload) × surface_mg / (accel_mg × (dry + ceiling))⌉)` in
/// checked `u128` — exact against the floored TWR (`⌊N·t/D⌋ ≥ 1200 ⇔ t ≥
/// ⌈1200·D/N⌉`). `None` for a coast drive, a zero divisor, or a floor above
/// [`MAX_THROTTLE_PERMILLE`] (the hull cannot land there at this load).
#[must_use]
pub fn least_landing_throttle(
    drive: &HullDrive,
    dry_kg: u64,
    payload_kg: u64,
    surface_mg: u32,
) -> Option<u32> {
    let DriveLaw::Flip { accel_mg, .. } = drive.law else {
        return None;
    };
    let per_throttle = u128::from(accel_mg)
        .checked_mul(u128::from(dry_kg).checked_add(u128::from(drive.payload_ceiling_kg))?)?;
    if per_throttle == 0 {
        return None;
    }
    let need = u128::from(LANDING_TWR_PERMILLE)
        .checked_mul(u128::from(dry_kg).checked_add(u128::from(payload_kg))?)?
        .checked_mul(u128::from(surface_mg))?;
    if need == 0 {
        return None;
    }
    let floor = u32::try_from(need.div_ceil(per_throttle)).ok()?;
    let floor = floor.max(MIN_THROTTLE_PERMILLE);
    (floor <= MAX_THROTTLE_PERMILLE).then_some(floor)
}

/// **The ascent Δv at one site end**, m/s (doc 20 cut SM.2, §3.3):
/// `√(surface_mg / 1000 × G0_MPS2 × radius_km × 1000)`, the circular speed at the
/// surface, ceiled to a whole m/s — the planner's one `f64` step here, frozen in
/// [`LegPlan::ascent_mps`]. Ceres 360, Vesta 254. `None` when it leaves `u32`.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn ascent_mps(surface_mg: u32, radius_km: u32) -> Option<u32> {
    let g = f64::from(surface_mg) / 1_000.0 * G0_MPS2;
    let r = f64::from(radius_km) * 1_000.0;
    let v = (g * r).sqrt().ceil();
    if !v.is_finite() || v < 0.0 || v > f64::from(u32::MAX) {
        return None;
    }
    Some(v as u32)
}

/// **A site end**: `Some((body, surface_mg, radius_km))` exactly when `place` is
/// a location of role [`crate::pack::SITE_ROLE`] whose body's entry carries both
/// surface figures (the validator guarantees it for a shipped site); a port end —
/// Ceres Freeport, every station — is `None` and pays nothing (the SM.2 brief's
/// decision 2).
pub(crate) fn site_end(pack: &RulesPack, place: &CanonicalId) -> Option<(CanonicalId, u32, u32)> {
    let Some(PackEntry::Location(location)) = pack.entry(place) else {
        return None;
    };
    if location.role.as_str() != crate::pack::SITE_ROLE {
        return None;
    }
    let body = location.body.clone()?;
    let Some(PackEntry::Body(def)) = pack.entry(&body) else {
        return None;
    };
    Some((body, def.surface_mg?, def.radius_km?))
}

/// The landing law at one site end: `Err(Landing)` under
/// [`LANDING_TWR_PERMILLE`], else the end's ascent Δv.
fn site_end_ascent(
    drive: &HullDrive,
    dry_kg: u64,
    payload_kg: u64,
    place: &CanonicalId,
    (body, surface_mg, radius_km): (CanonicalId, u32, u32),
) -> Result<u32, PlanError> {
    let twr = landing_twr_permille(drive, dry_kg, payload_kg, surface_mg).unwrap_or(0);
    if twr < LANDING_TWR_PERMILLE {
        return Err(PlanError::Landing {
            place: place.to_string(),
            body: body.to_string(),
            twr_permille: twr,
        });
    }
    ascent_mps(surface_mg, radius_km).ok_or(PlanError::Overflow)
}

/// **The least throttle a listed route can fly under the landing law at this
/// load** (doc 20 cut SM.2): the largest [`least_landing_throttle`] over the
/// route's site ends (its `from` and every leg's `to` that is a site),
/// [`MIN_THROTTLE_PERMILLE`] when it has none — so a non-site route's floor is the
/// band's own.
///
/// # Errors
///
/// [`PlanError::Landing`] (with the TWR at full throttle) when some site end
/// cannot be reached anywhere in the band; [`PlanError::UnknownRoute`] as
/// [`plan_passage`].
pub fn route_landing_floor(
    pack: &RulesPack,
    route: &CanonicalId,
    drive: &HullDrive,
    dry_kg: u64,
    payload_kg: u64,
) -> Result<u32, PlanError> {
    let Some(PackEntry::Route(entry)) = pack.entry(route) else {
        return Err(PlanError::UnknownRoute {
            route: route.to_string(),
        });
    };
    let mut floor = MIN_THROTTLE_PERMILLE;
    let places = std::iter::once(&entry.from).chain(entry.legs.iter().map(|leg| &leg.to));
    for place in places {
        let Some((body, surface_mg, _)) = site_end(pack, place) else {
            continue;
        };
        let Some(least) = least_landing_throttle(drive, dry_kg, payload_kg, surface_mg) else {
            let full = drive.clone().at_throttle(MAX_THROTTLE_PERMILLE);
            return Err(PlanError::Landing {
                place: place.to_string(),
                body: body.to_string(),
                twr_permille: landing_twr_permille(&full, dry_kg, payload_kg, surface_mg)
                    .unwrap_or(0),
            });
        };
        floor = floor.max(least);
    }
    Ok(floor)
}

/// **The leg home a departure for a site holds back** (doc 20 cut SM.2, decision 7): the
/// largest burn and crew-days of the passage home over the window, planned at the least
/// throttle that lifts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HomeReserve {
    /// The home burn, tonnes (the most over the window).
    pub burn_t: u64,
    /// The home crew-days (the most over the window).
    pub crew_days: u32,
    /// The throttle it is planned at: the home routes' least landing floor.
    pub throttle: u32,
}

/// **The leg home over the window, pure** (doc 20 cut SM.2, decision 7; factored out of
/// `voyage.rs`'s `site_reserve` by the SM.2 fixer so the landing suite's year sweep reads the
/// production function): the throttle is the least [`route_landing_floor`] over the home
/// `routes` that answer at `home_payload_kg` (the least throttle that lifts, so the least burn
/// that can leave); the home routes are planned at `arrive_ic + k × DAY_SECS` for `k` in
/// `0..=window_days` when the first plan crosses bodies (a same-body hop does not move with
/// the sky, so `k = 0` alone), and the largest burn and crew-days are kept.
///
/// # Errors
///
/// [`PlanError::Landing`] when no home route lifts at `home_payload_kg` (the first route's
/// refusal), [`PlanError::Overflow`] on an instant past `i64`, and any other planner
/// refusal.
#[allow(clippy::too_many_arguments)] // the planner's inputs, kept whole
pub fn home_reserve(
    sky: &Sky,
    pack: &RulesPack,
    routes: &[CanonicalId],
    drive: &HullDrive,
    dry_kg: u64,
    home_payload_kg: u64,
    occupants: u32,
    arrive_ic: i64,
    window_days: i64,
) -> Result<HomeReserve, PlanError> {
    let mut floor: Option<u32> = None;
    let mut first_refusal: Option<PlanError> = None;
    for route in routes {
        match route_landing_floor(pack, route, drive, dry_kg, home_payload_kg) {
            Ok(least) => floor = Some(floor.map_or(least, |f| f.min(least))),
            Err(error) => {
                first_refusal.get_or_insert(error);
            }
        }
    }
    let Some(throttle) = floor else {
        return Err(first_refusal.unwrap_or(PlanError::UnknownRoute {
            route: "(none listed)".to_owned(),
        }));
    };
    let home_drive = drive.clone().at_throttle(throttle);
    let plan_at = |k: i64| -> Result<PassagePlan, PlanError> {
        let at = k
            .checked_mul(i64::from(DAY_SECS))
            .and_then(|offset| arrive_ic.checked_add(offset))
            .ok_or(PlanError::Overflow)?;
        choose_route(
            sky,
            pack,
            routes,
            &home_drive,
            dry_kg,
            home_payload_kg,
            occupants,
            at,
            RoutePolicy::default(),
            None,
        )
    };
    let first = plan_at(0)?;
    let mut reserve = HomeReserve {
        burn_t: first.burn_t,
        crew_days: first.crew_days,
        throttle,
    };
    if first.legs.iter().any(|leg| leg.dist_km > 0) {
        for k in 1..=window_days {
            let plan = plan_at(k)?;
            reserve.burn_t = reserve.burn_t.max(plan.burn_t);
            reserve.crew_days = reserve.crew_days.max(plan.crew_days);
        }
    }
    Ok(reserve)
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
///
/// **Two moons of one parent are measured in the parent's frame** (doc 19 cut
/// A.8, D-19-21): each body less its primary at its own instant, no keep-out.
/// Both ride with the planet, so in the Sun's frame Mars's 24 km/s of orbital
/// motion would turn the 29,500 km Deimos–Phobos hop into a day and a half and
/// five tonnes; in Mars's frame it is the hours doc 10 1A gives a nearby moon.
/// A same-instant chord is frame-invariant, so every authored `base_s` stands.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn leg_distance_km(
    sky: &Sky,
    a: &CanonicalId,
    ta: i64,
    b: &CanonicalId,
    tb: i64,
) -> Option<u64> {
    if let (Some(pa), Some(pb)) = (sky.parent_of(a), sky.parent_of(b))
        && pa == pb
    {
        let primary = pa.clone();
        let rel = |body: PosKm, parent: PosKm| PosKm {
            x: body.x.saturating_sub(parent.x),
            y: body.y.saturating_sub(parent.y),
            z: body.z.saturating_sub(parent.z),
        };
        let from = rel(sky.position_km(a, ta)?, sky.position_km(&primary, ta)?);
        let to = rel(sky.position_km(b, tb)?, sky.position_km(&primary, tb)?);
        return Some(from.distance_km(to));
    }
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
    let mut from_place = &entry.from;
    let mut t0 = depart_ic;
    for leg in &entry.legs {
        let to = body_of(sky, pack, &leg.to)?;
        // Doc 20 cut SM.2 (D-20-3; the SM.2 brief's lead rulings 3 and 4): only a
        // leg one of whose ends is a site meets the landing law, and only a site
        // end pays the ascent — one statement, no other end pays. The seconds are
        // unchanged: a same-body hop is its `base_s` through the laden step.
        let mut ascent: u32 = 0;
        for place in [from_place, &leg.to] {
            if let Some(end) = site_end(pack, place) {
                let paid = site_end_ascent(drive, dry_kg, payload_kg, place, end)?;
                ascent = ascent.checked_add(paid).ok_or(PlanError::Overflow)?;
            }
        }
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
            ascent_mps: ascent,
        });
        from = to;
        from_place = &leg.to;
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
        landed_crew_days: 0,
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
        // Doc 20 cut SM.2 (§3.3): a site end's ascent is Δv the leg pays whatever
        // its distance (a same-body hop is `dist_km` 0 and pays only this).
        dv += f64::from(leg.ascent_mps);
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
            let tonnes = tonnes as u64;
            // Doc 20 cut SM.2 (§3.3): a leg touching a site burns at least a
            // tonne, after the ceil.
            if legs.iter().any(|leg| leg.ascent_mps > 0) {
                return Ok(tonnes.max(SITE_MIN_BURN_T));
            }
            Ok(tonnes)
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

/// **One rig in a hull's bay, as the payload counts it** (doc 20 §4.3, D-20-7;
/// cut SM.3a): the terrain vehicle, its dry mass (its spec's `dry_mass_t` ×
/// 1,000), its hopper vault and every good its hopper accepts. Frozen into the
/// filed passage (`PendingVoyage.carried`) because the burn's prepare cannot
/// read a placement, which is a legacy slot an `ActionTxn` cannot reach
/// (`mush-sim/src/journey.rs`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarriedRig {
    /// The terrain vehicle, `ObjId.0`.
    pub tv: u32,
    /// Its dry mass, kilograms.
    pub dry_kg: u64,
    /// Its hopper vault, `VaultId.0`; `None` reads as an empty hopper.
    pub hopper: Option<u64>,
    /// The goods its hopper accepts, in goods-table order.
    pub goods: Vec<CanonicalId>,
}

/// **What a bay's rigs weigh** (doc 20 §4.3, D-20-7; cut SM.3a): Σ of every
/// rig's `dry_kg`, plus — for a rig with a hopper — Σ over its `goods` of
/// `read(hopper, good) × mass_kg_per_unit`, checked. Added to [`payload_kg`] by
/// `+ship/plan`, `+ship/undock` and the burn's prepare, so a Dray-P with two
/// rigs aboard plans exactly as a Dray at equal total payload. A rig is placed
/// in the bay space, never the airlock, so it is never counted among the souls
/// aboard.
///
/// # Errors
///
/// [`PlanError::Unreadable`] for a good the table does not weigh, `read`'s own
/// refusal, and [`PlanError::Overflow`].
pub fn carried_kg(
    goods: &GoodsTable,
    rigs: &[CarriedRig],
    mut read: impl FnMut(u64, &CanonicalId) -> Result<u64, PlanError>,
) -> Result<u64, PlanError> {
    let mut total: u64 = 0;
    for rig in rigs {
        total = total.checked_add(rig.dry_kg).ok_or(PlanError::Overflow)?;
        let Some(hopper) = rig.hopper else {
            continue;
        };
        for good in &rig.goods {
            let per_unit = goods
                .good(good)
                .and_then(crate::definitions::GoodDef::mass_kg_per_unit)
                .ok_or_else(|| PlanError::Unreadable {
                    what: format!("the mass of {good}"),
                })?;
            let qty = read(hopper, good)?;
            total = qty
                .checked_mul(per_unit)
                .and_then(|kg| total.checked_add(kg))
                .ok_or(PlanError::Overflow)?;
        }
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
            ascent_mps: 0,
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
                ascent_mps: 0,
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
            ascent_mps: 0,
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

    /// **Doc 20 cut SM.2 (D-20-3)**: the thrust-to-weight is an integer permille,
    /// floored, exact either side of the line; the least landing throttle is the
    /// exact ceiling against it; the ascent is the one `f64` step, ceiled; a coast
    /// drive has no thrust figure; a site hop burns at least a tonne.
    #[test]
    fn sm2_landing_twr_and_ascent_are_integers_where_they_can_be() {
        let drive = flip();
        // Ceres laden (the rail): 50 / 28 of a g, 1,785 permille.
        assert_eq!(
            landing_twr_permille(&drive, 240_000, 181_000, 28),
            Some(1_785)
        );
        assert_eq!(
            landing_twr_permille(&drive.clone().at_throttle(672), 240_000, 181_000, 28),
            Some(1_200)
        );
        assert_eq!(
            landing_twr_permille(&drive.clone().at_throttle(671), 240_000, 181_000, 28),
            Some(1_198)
        );
        assert_eq!(
            least_landing_throttle(&drive, 240_000, 181_000, 28),
            Some(672)
        );
        assert_eq!(
            least_landing_throttle(&drive, 240_000, 181_000, 25),
            Some(600)
        );
        // Luna at the rail: under the line at full thrust, so no throttle lands.
        assert_eq!(
            landing_twr_permille(&drive, 240_000, 181_000, 165),
            Some(303)
        );
        assert_eq!(least_landing_throttle(&drive, 240_000, 181_000, 165), None);
        // A tiny body is floored at the band's own least throttle.
        assert_eq!(
            least_landing_throttle(&drive, 240_000, 0, 1),
            Some(MIN_THROTTLE_PERMILLE)
        );
        // A coast drive has no thrust figure; a zero gravity has no weight.
        assert_eq!(landing_twr_permille(&coast(), 240_000, 0, 28), None);
        assert_eq!(least_landing_throttle(&coast(), 240_000, 0, 28), None);
        assert_eq!(landing_twr_permille(&drive, 240_000, 0, 0), None);
        // The ascent: Ceres sqrt(129,055.5) = 359.24 → 360; Vesta 253.93 → 254.
        assert_eq!(ascent_mps(28, 470), Some(360));
        assert_eq!(ascent_mps(25, 263), Some(254));
        assert_eq!(ascent_mps(0, 470), Some(0));
        // A same-body hop with one site end: Δv is the ascent alone, a few
        // kilograms, burned as a whole tonne; without the ascent it burns nothing.
        let hop = |ascent: u32| LegPlan {
            ascent_mps: ascent,
            ..leg(5_322)
        };
        assert_eq!(passage_burn(&drive, 240_000, 86_580, &[hop(360)]), Ok(1));
        assert_eq!(passage_burn(&drive, 240_000, 86_580, &[hop(0)]), Ok(0));
        let profile = flip_profile(&drive, 240_000, 86_580, &[hop(360)]).expect("a profile");
        assert!((profile.dv_mps - 360.0).abs() < 1e-9, "{}", profile.dv_mps);
    }

    /// **Doc 20 cut SM.2**: a route's landing floor is the largest least landing
    /// throttle over its site ends, the band's floor with none, and a Landing
    /// refusal at full thrust's TWR when a site end cannot be reached.
    #[test]
    fn sm2_route_landing_floor() {
        let Ok(pack) = crate::pack::load_pack(crate::definitions::content::LANE_JSON) else {
            panic!("the shipped lane loads");
        };
        let drive = flip();
        let floor = |route: &str, payload: u64| {
            route_landing_floor(&pack, &cid(route), &drive, 240_000, payload)
        };
        // Ceres's hops at the rail: 672; Rheasilvia (Vesta): 600; a port-only route
        // and the inter-moon hop: the band's 250.
        assert_eq!(floor("ceres_ahuna", 181_000), Ok(672));
        assert_eq!(floor("ahuna_ceres", 181_000), Ok(672));
        assert_eq!(floor("ceres_rheasilvia", 181_000), Ok(600));
        assert_eq!(floor("ceres_stickney", 181_000), Ok(MIN_THROTTLE_PERMILLE));
        assert_eq!(
            floor("voltaire_stickney", 181_000),
            Ok(MIN_THROTTLE_PERMILLE)
        );
        // Empty, an empty hull on Ceres still needs 384 permille
        // (1200 x 240,000 x 28 / (50 x 421,000) = 383.09, ceiled).
        assert_eq!(floor("ceres_ahuna", 0), Ok(384));
        assert_eq!(
            floor("nowhere", 0),
            Err(PlanError::UnknownRoute {
                route: "nowhere".to_owned()
            })
        );
        // A coast drive cannot land anywhere: refused at TWR 0.
        assert_eq!(
            route_landing_floor(&pack, &cid("ceres_ahuna"), &coast(), 240_000, 0),
            Err(PlanError::Landing {
                place: "ahuna_flank".to_owned(),
                body: "ceres".to_owned(),
                twr_permille: 0,
            })
        );
    }
}
