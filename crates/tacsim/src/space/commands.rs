//! **Authoritative `+helm` / `+tac` Verbs, `mush-sim` Standing-Order Channels &
//! `mush-propagation` Light-Lag / IR Detection (`SF.R3`)**
//! (`docs/design/35-ship-free-flight-combat-and-mining.md` §5, §6.2, `D-35-9`, `D-35-10`,
//! `D-35-16`, `docs/design/35a-space-flight-review.md` §3, §8.3).
//!
//! Implements the player-facing `+helm` and `+tac` commands on the `Authoritative` command lane:
//! 1. **Physical aboard & bridge seat gating (`D-35-9`, `35a` §3 #1–#2)**:
//!    Every `+helm` and `+tac` switch resolves the carrier hull the caller is physically aboard
//!    ([`hull_aboard_for_actor`]). Off-ship callers are refused (`"You are not aboard a ship."`).
//!    Station assignments live on `mush-sim`'s [`SpatialState`] (`stations:<hull>`,
//!    `assign:<hull>`, `cgen:<hull>`) via [`mush_sim::declare_station`],
//!    [`mush_sim::commands::assign_station`], and [`mush_sim::vacate_station`], which publish
//!    [`mush_core::ChannelGrant`]s onto [`World::command_subjects_of`]. Unrostered passengers or
//!    stowaways aboard a hull hold no seat and are refused (`"You hold no bridge station on this ship."`).
//! 2. **`mush-sim` standing-order channels (`D-35-9`)**:
//!    Continuous helm controls (`+helm/heading`, `+helm/thrust`, `+helm/emcon`) validate and
//!    record [`mush_sim::tick::StandingOrder`] records on [`OrderChannel::Heading`],
//!    [`OrderChannel::Thrust`], and [`OrderChannel::Posture`] in [`SpaceBubbleBook`] and apply
//!    the corresponding [`BubbleInputCommand`] to the hull's active [`LocalSpaceBubble`] input
//!    tape ([`apply_bubble_command`]).
//! 3. **`mush-propagation` light-lag & IR detection (`D-35-10`)**:
//!    [`evaluate_space_contact_detection`] evaluates each contact in the bubble through
//!    [`mush_propagation::resolve_detection`] over [`Medium::Vacuum`] using inverse-square range
//!    scaling, thermal/IR bloom (`EmconPosture` floor + radiator heat + torch thrust + weapon
//!    flash), and observer sensor aperture (`SubsystemCrit::SensorsBlinded`). Undetected contacts
//!    ([`DetectionOutcome::Missed`]) are omitted from `+tac/scan` and panel snapshots
//!    (`space.holotank`, `space.cockpit`), and targeting an undetected contact with `+tac/lock`,
//!    `+tac/beam`, `+tac/torp`, or `+tac/railgun` refuses with `"No such contact."`.

use std::collections::BTreeSet;

use mush_core::output::display_name;
use mush_core::{Audience, ChannelId, ClockRef, ClockStamp, CmdError, Ctx, ObjId, World};
use mush_propagation::{
    DetectionOutcome, Geometry, Interference, Medium, PropagationContext, resolve_detection,
};
use mush_proto::{OutputEvent, PanelFrameKind};
use mush_sim::tick::{MAX_ORDER_SCALAR, MIN_ORDER_SCALAR, OrderChannel, OrderValue, StandingOrder};
use mush_sim::{DomainId, JourneyBook, SpatialState, StationId};

use super::bubble::{
    BubbleInputCommand, EmconPosture, HullStatus, LocalSpaceBubble, SpaceRelation, SpaceShipState,
    SubsystemCrit,
};
use super::tick::{
    ActiveInterdictionBubble, FreeFlightGuidanceMode, SpaceBubbleBook, SpaceFreeFlightRecord,
    apply_bubble_command,
};
use super::weapons::LaserMode;

/// The helm command verb (`+helm`).
pub const HELM_VERB: &str = "+helm";

/// All switches supported by `+helm`.
pub const HELM_SWITCHES: [&str; 10] = [
    "status",
    "station",
    "heading",
    "thrust",
    "emcon",
    "roll",
    "guidance",
    "decouple",
    "burn",
    "autopilot",
];

/// Help text for `+helm`.
pub const HELM_HELP: &str = "\
+helm  --  Ship helm, bridge stations, and standing orders (doc 35 §5, D-35-6, D-35-9).
+helm[/status]                        Show bridge stations, standing orders, and flight state.
+helm/station <seat>[=<crew>]         Assign an aboard crew member (or yourself) to a bridge station
                                      (command|helm|tactical|ew|engineering|flight_ops), or `off`.
+helm/heading <yaw_deg>[,<pitch_deg>] Set nose attitude and write StandingOrder::Heading.
+helm/thrust <permille>[,<x>,<y>,<z>] [force]
                                      Set main-drive throttle (0..1000) and write StandingOrder::Thrust.
+helm/emcon <ghost|passive|standard|combat>
                                      Set emission posture and write StandingOrder::Posture.
+helm/roll <roll_deg>                 Set roll angle in degrees (-180..180).
+helm/guidance <attitude_deg|mode>    Set guidance yaw (0..359) or mode (prograde|retrograde|intercept|match).
+helm/decouple                        Decouple from corridor transit into 3D heliocentric free flight.
+helm/burn <ax>,<ay>,<az> [<s_ic>] [force]
                                      Execute a timed reaction-mass burn (m/s²) against stow.propellant.
+helm/autopilot <port>                Plan and recouple onto a corridor passage from live (pos, vel).";

/// The tactical command verb (`+tac`).
pub const TAC_VERB: &str = "+tac";

/// All switches supported by `+tac`.
pub const TAC_SWITCHES: [&str; 7] = ["status", "scan", "lock", "beam", "torp", "railgun", "pdc"];

/// Help text for `+tac`.
pub const TAC_HELP: &str = "\
+tac  --  Ship tactical sensors, fire control, and point defence (doc 35 §6, D-35-9, D-35-10).
+tac[/status]                         Show tactical weapon mounts, locks, and sensor status.
+tac/scan                             Run passive/active IR sensor pass with light-lag (mush-propagation).
+tac/lock <contact|off>               Acquire or clear fire-control lock on a detected contact.
+tac/beam [<contact>]                 Fire offensive laser mounts at a detected hostile contact.
+tac/torp [<contact>[=<count>]]       Launch torpedo salvo (default 1) at a detected hostile contact.
+tac/railgun [<contact>]              Fire spinal/turret railguns at a detected hostile contact.
+tac/pdc <auto|manual|point|offense>  Configure point-defence autocannons or dual-mode laser mounts.";

/// Feed identifier for the 3D holotank tactical display (`SF.2`, `SF.7`).
pub const HOLOTANK_FEED: &str = "space.holotank";

/// Feed identifier for the first-person canopy cockpit display (`SF.2`, `SF.7`).
pub const SPACE_COCKPIT_FEED: &str = "space.cockpit";

/// Canonical refusal when the caller is not physically aboard any ship.
pub const NOT_ABOARD_SHIP: &str = "You are not aboard a ship.";

/// Canonical refusal when an aboard caller holds no bridge station on the ship.
pub const NO_BRIDGE_STATION: &str = "You hold no bridge station on this ship.";

/// Canonical refusal when a target contact is not detected by `mush-propagation`.
pub const NO_SUCH_CONTACT: &str = "No such contact.";

/// Speed of light in vacuum ($c = 299{,}792{,}458\text{ m/s}$).
pub const SPEED_OF_LIGHT_MPS: u64 = 299_792_458;

/// Canonical six-seat bridge role taxonomy (`doc 35` §5.2, `D-35-9`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BridgeRole {
    /// Command seat (`command`): full bridge authority across all channels.
    Command,
    /// Helm seat (`helm`): attitude, thrust, and trajectory (`movement`).
    Helm,
    /// Tactical seat (`tactical`): sensors, target locks, and weapons (`sensors`, `fire`).
    Tactical,
    /// Electronic warfare / sensors seat (`ew`): EMCON and sensor passes (`sensors`).
    Ew,
    /// Engineering seat (`engineering`): heat and damage control (`damage_control`).
    Engineering,
    /// Flight operations seat (`flight_ops`): drone/mining bay operations (`flight_ops`).
    FlightOps,
}

impl BridgeRole {
    /// Station identifier token on `mush-sim`'s [`SpatialState`].
    #[must_use]
    pub const fn station_token(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Helm => "helm",
            Self::Tactical => "tactical",
            Self::Ew => "ew",
            Self::Engineering => "engineering",
            Self::FlightOps => "flight_ops",
        }
    }

    /// Role string stored in `mush-sim`'s [`mush_sim::StationSpec`].
    #[must_use]
    pub const fn role_name(self) -> &'static str {
        self.station_token()
    }

    /// Priority assigned to the station in `mush-sim`.
    #[must_use]
    pub const fn priority(self) -> u16 {
        match self {
            Self::Command => 3,
            Self::Helm | Self::Tactical => 2,
            Self::Ew | Self::Engineering | Self::FlightOps => 1,
        }
    }

    /// Channel names granted by this bridge station (`D-35-9`).
    #[must_use]
    pub const fn channel_names(self) -> &'static [&'static str] {
        match self {
            Self::Command => &[
                "movement",
                "sensors",
                "fire",
                "damage_control",
                "flight_ops",
            ],
            Self::Helm => &["movement"],
            Self::Tactical => &["sensors", "fire"],
            Self::Ew => &["sensors"],
            Self::Engineering => &["damage_control"],
            Self::FlightOps => &["flight_ops"],
        }
    }

    /// Build the [`BTreeSet<ChannelId>`] granted by this station.
    #[must_use]
    pub fn channel_set(self) -> BTreeSet<ChannelId> {
        self.channel_names()
            .iter()
            .filter_map(|name| ChannelId::new(*name))
            .collect()
    }

    /// Return whether this bridge role grants the named control channel.
    #[must_use]
    pub fn grants_channel(self, channel: &str) -> bool {
        self.channel_names().contains(&channel)
    }

    /// Parse a station token into a [`BridgeRole`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "command" | "captain" | "cmd" => Some(Self::Command),
            "helm" | "pilot" => Some(Self::Helm),
            "tactical" | "tac" | "gunner" => Some(Self::Tactical),
            "ew" | "sensors" | "comms" => Some(Self::Ew),
            "engineering" | "eng" => Some(Self::Engineering),
            "flight_ops" | "flight" | "ops" => Some(Self::FlightOps),
            _ => None,
        }
    }
}

/// Resolve the carrier hull [`ObjId`] that `actor` is physically aboard (`D-35-9`, `35a` §3 #1).
///
/// Checks both port-registered hulls (`crate::port::ship::hull_of_room` +
/// `crate::port::voyage::aboard`) and any carrier hull in [`SpatialState`] /
/// [`SpaceBubbleBook`] via `crate::detachment::planning::projection::physically_aboard`.
#[must_use]
pub fn hull_aboard_for_actor(w: &World, actor: ObjId) -> Option<ObjId> {
    let room = w.get(actor)?.location?;
    if let Some((view, _)) = crate::port::ship::hull_of_room(w, room)
        && crate::port::voyage::aboard(w, view.hull(), actor)
    {
        return Some(view.hull());
    }
    if let Some(book) = w.module_state::<SpaceBubbleBook>() {
        for hull in book.by_hull.keys().copied() {
            if crate::detachment::planning::projection::physically_aboard(w, actor, hull) {
                return Some(hull);
            }
        }
    }
    if let Some(sim) = w.module_state::<SpatialState>()
        && let Some(anchor) = sim.spaces.get(&room)
        && let mush_sim::SpaceAnchor::MovingObject { host } = anchor.anchor
        && crate::detachment::planning::projection::physically_aboard(w, actor, host)
    {
        return Some(host);
    }
    None
}

/// Return whether `actor` is the legal captain/principal of `hull` (Co-op charter holder,
/// graduated title holder via [`crate::port::coop::helm_of`], or yard dispatcher).
#[must_use]
pub fn is_hull_captain(w: &World, hull: ObjId, actor: ObjId) -> bool {
    if crate::port::coop::helm_of(w, actor).is_some_and(|c| c.hull == hull) {
        return true;
    }
    crate::port::yard::is_dispatcher_of(w, hull, actor)
}

/// Resolve the [`BridgeRole`] held by `actor` on `hull` (`D-35-9`, `35a` §3 #2).
///
/// 1. Checks `mush-sim`'s [`SpatialState::assignments`] for an explicit bridge seat held by
///    `actor` on `hull`.
/// 2. If `actor` holds no explicit seat in `SpatialState::assignments`, grants
///    [`BridgeRole::Command`] **only** when `is_hull_captain(w, hull, actor)` is true and the
///    `command` station on `hull` is not occupied by another crew member.
/// 3. Unrostered passengers or stowaways aboard `hull` receive `None`.
#[must_use]
pub fn seat_role_for_actor(w: &World, hull: ObjId, actor: ObjId) -> Option<BridgeRole> {
    if let Some(sim) = w.module_state::<SpatialState>()
        && let Some(assignments) = sim.assignments.get(&hull)
    {
        for (station_id, seat) in &assignments.seats {
            if seat.actor == actor
                && let Some(role) = BridgeRole::parse(station_id.as_str())
            {
                return Some(role);
            }
        }
    }
    if is_hull_captain(w, hull, actor) {
        let command_taken_by_other = w
            .module_state::<SpatialState>()
            .and_then(|sim| sim.assignments.get(&hull))
            .and_then(|a| StationId::new("command").and_then(|sid| a.seats.get(&sid)))
            .is_some_and(|seat| seat.actor != actor);
        if !command_taken_by_other {
            return Some(BridgeRole::Command);
        }
    }
    None
}

/// Require that `ctx.actor` is physically aboard a ship and holds a bridge station on it
/// that grants `required_channel` (when `Some`).
fn require_aboard_and_seat(
    ctx: &Ctx<'_>,
    required_channel: Option<&str>,
) -> Result<(ObjId, BridgeRole), CmdError> {
    let hull =
        hull_aboard_for_actor(ctx.w, ctx.actor).ok_or_else(|| CmdError::msg(NOT_ABOARD_SHIP))?;
    let role = seat_role_for_actor(ctx.w, hull, ctx.actor)
        .ok_or_else(|| CmdError::msg(NO_BRIDGE_STATION))?;
    if let Some(channel) = required_channel
        && !role.grants_channel(channel)
    {
        return Err(CmdError::msg(format!(
            "Your bridge station ({}) does not control the {channel} channel.",
            role.station_token()
        )));
    }
    Ok((hull, role))
}

/// Resolve the active `(DomainId, generation, tick)` for recording a `mush-sim` [`StandingOrder`]
/// on `hull`.
fn domain_stamp_for_hull(w: &World, hull: ObjId) -> (DomainId, u64, i64) {
    let ic_at = w.world_ic_now().at.max(1);
    if let Some(book) = w.module_state::<SpaceBubbleBook>()
        && let Some(active) = book.active_for_hull(hull)
    {
        return (active.domain, 1, ic_at);
    }
    if let Some(jbook) = w.module_state::<JourneyBook>()
        && let Some(journey) = jbook.get(hull)
        && let mush_sim::JourneyState::Promoted { domain } = journey.state()
    {
        return (*domain, 1, ic_at);
    }
    if let Some((domain, _)) = mush_sim::pose_of(w, hull) {
        return (domain, 1, ic_at);
    }
    (DomainId(1), 1, ic_at)
}

/// Convert `(yaw_deg, pitch_deg)` into a `mush-sim` [`OrderValue::Vector`] with components in
/// `MIN_ORDER_SCALAR..=MAX_ORDER_SCALAR` (`-1000..=1000`).
#[must_use]
pub fn heading_order_vector(yaw_deg: i32, pitch_deg: i32) -> OrderValue {
    let yaw_rad = (yaw_deg as f64).to_radians();
    let pitch_rad = (pitch_deg as f64).to_radians();
    let cos_p = libm::cos(pitch_rad);
    let x = (libm::cos(yaw_rad) * cos_p * 1_000.0).round() as i32;
    let y = (libm::sin(yaw_rad) * cos_p * 1_000.0).round() as i32;
    let z = (libm::sin(pitch_rad) * 1_000.0).round() as i32;
    OrderValue::Vector {
        x: x.clamp(MIN_ORDER_SCALAR, MAX_ORDER_SCALAR),
        y: y.clamp(MIN_ORDER_SCALAR, MAX_ORDER_SCALAR),
        z: z.clamp(MIN_ORDER_SCALAR, MAX_ORDER_SCALAR),
    }
}

/// Map `mush-sim`'s [`OrderValue::Emcon`] level (`0..=3`) to [`EmconPosture`].
#[must_use]
pub const fn emcon_posture_for_level(level: u8) -> EmconPosture {
    match level {
        0 => EmconPosture::Ghost,
        1 => EmconPosture::Passive,
        2 => EmconPosture::Standard,
        _ => EmconPosture::ActiveCombat,
    }
}

/// Map [`EmconPosture`] to `mush-sim`'s [`OrderValue::Emcon`] level (`0..=3`).
#[must_use]
pub const fn emcon_level_for_posture(posture: EmconPosture) -> u8 {
    match posture {
        EmconPosture::Ghost => 0,
        EmconPosture::Passive => 1,
        EmconPosture::Standard => 2,
        EmconPosture::ActiveCombat => 3,
    }
}

/// Parse an EMCON token (`ghost`/`silent`/`0`, `passive`/`reduced`/`1`,
/// `standard`/`active`/`nominal`/`2`, `combat`/`beacon`/`full`/`3`).
#[must_use]
pub fn parse_emcon_token(token: &str) -> Option<(u8, EmconPosture)> {
    let folded = token.trim().to_ascii_lowercase();
    let level = match folded.as_str() {
        "0" | "ghost" | "silent" => 0,
        "1" | "passive" | "reduced" => 1,
        "2" | "standard" | "std" | "active" | "nominal" => 2,
        "3" | "combat" | "active_combat" | "beacon" | "full" => 3,
        _ => return None,
    };
    Some((level, emcon_posture_for_level(level)))
}

// ---------------------------------------------------------------------------
// Light-lag & IR detection through `mush-propagation` (D-35-10)
// ---------------------------------------------------------------------------

/// Compute one-way vacuum light propagation delay in milliseconds for `dist_m` metres.
#[must_use]
pub const fn vacuum_light_lag_ms(dist_m: u64) -> u64 {
    ((dist_m as u128) * 1_000 / (SPEED_OF_LIGHT_MPS as u128)) as u64
}

/// Effective observer sensor aperture in permille (`0..=1250`), accounting for
/// `SubsystemCrit::SensorDegraded` and observer `EmconPosture`.
#[must_use]
pub fn observer_sensor_aperture_permille(observer: &SpaceShipState) -> u32 {
    if observer.crits.contains(&SubsystemCrit::SensorDegraded) {
        return 100;
    }
    match observer.emcon {
        EmconPosture::Ghost => 650,
        EmconPosture::Passive => 900,
        EmconPosture::Standard => 1_000,
        EmconPosture::ActiveCombat => 1_250,
    }
}

/// Result of evaluating a target ship's detectability from an observer ship via
/// `mush-propagation` (`D-35-10`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceContactObservation {
    /// Target ship identifier in the bubble.
    pub ship_id: String,
    /// Target ship callsign.
    pub callsign: String,
    /// True 3D separation in metres.
    pub dist_m: i64,
    /// One-way light propagation delay in milliseconds ([`vacuum_light_lag_ms`]).
    pub light_lag_ms: u64,
    /// Effective thermal/IR signature fed into [`mush_propagation::resolve_detection`].
    pub ir_signature: u32,
    /// Target's current emission posture.
    pub emcon: EmconPosture,
    /// `mush-propagation` detection outcome (`Detected` or `Missed`).
    pub outcome: DetectionOutcome,
}

/// Evaluate whether `observer` detects `target` across vacuum using
/// [`mush_propagation::resolve_detection`] (`D-35-10`).
///
/// Thermal/IR signature combines:
/// - Base EMCON posture floor (`Ghost` = 2, `Passive` = 30, `Standard` = 220, `ActiveCombat` = 1,500)
/// - Radiator heat bloom (`max(0, heat_permille - 80) / 2`)
/// - Torch exhaust plume (`|thrust_mps2| * 180`, or `+400` whenever burning)
/// - Active weapon discharge flash (`+600` while any laser/railgun mount is cycling)
///
/// Detection scales inversely with distance squared (`(dist_m / 10_000)²`, i.e. 10 km units
/// squared) and scales directly with the observer's effective sensor aperture
/// ([`observer_sensor_aperture_permille`]).
#[must_use]
pub fn evaluate_space_contact_detection(
    observer: &SpaceShipState,
    observer_obj: ObjId,
    target: &SpaceShipState,
    target_obj: ObjId,
    domain: Option<DomainId>,
    at_ic: i64,
) -> SpaceContactObservation {
    let dx = i128::from(target.pos_m[0]) - i128::from(observer.pos_m[0]);
    let dy = i128::from(target.pos_m[1]) - i128::from(observer.pos_m[1]);
    let dz = i128::from(target.pos_m[2]) - i128::from(observer.pos_m[2]);
    let dist_sq_m2 = dx * dx + dy * dy + dz * dz;
    let dist_m = (libm::sqrt(dist_sq_m2 as f64).round() as i64).max(0);
    let lag_ms = vacuum_light_lag_ms(u64::try_from(dist_m).unwrap_or(0));

    let base_ir: u32 = match target.emcon {
        EmconPosture::Ghost => 2,
        EmconPosture::Passive => 30,
        EmconPosture::Standard => 220,
        EmconPosture::ActiveCombat => 1_500,
    };
    let excess_heat = target.heat_permille.saturating_sub(80);
    let heat_bloom = u32::from(excess_heat / 2);
    let thrust_sq = i128::from(target.thrust_mps2[0]).pow(2)
        + i128::from(target.thrust_mps2[1]).pow(2)
        + i128::from(target.thrust_mps2[2]).pow(2);
    let thrust_mag = libm::sqrt(thrust_sq as f64).round() as u32;
    let thrust_bloom = if target.is_burning() {
        400u32.saturating_add(thrust_mag.saturating_mul(180))
    } else {
        0
    };
    let weapon_flash = if target.lasers.iter().any(|l| l.cooldown_remaining_ms > 0)
        || target.railguns.iter().any(|r| r.cooldown_remaining_ms > 0)
    {
        600
    } else {
        0
    };
    let ir_signature = base_ir
        .saturating_add(heat_bloom)
        .saturating_add(thrust_bloom)
        .saturating_add(weapon_flash);

    let aperture_permille = observer_sensor_aperture_permille(observer);
    let scaled_signature = ir_signature.saturating_mul(aperture_permille) / 1_000;

    // Inverse-square range term in (10 km)² units (`(dist_m / 10_000)²`, minimum 1).
    // At 30 km (interdiction standoff): `range_unit = 3`, `range_sq_term = 9`, threshold = `1 + 9 = 10`.
    // - A cold `Ghost` ship (`ir_signature = 2 < 10`) is `Missed`!
    // - A `Standard` ship (`ir_signature = 220 >= 10`) or burning/hot/firing `Ghost` ship is `Detected`!
    let range_10km = (dist_m / 10_000).max(1);
    let range_sq_term = u32::try_from(
        i128::from(range_10km)
            .saturating_mul(i128::from(range_10km))
            .min(i128::from(u32::MAX / 2)),
    )
    .unwrap_or(u32::MAX / 2);

    let stamp = ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: at_ic.max(1),
    };
    let prop_ctx = PropagationContext {
        domain,
        geometry: Geometry {
            near: observer_obj,
            far: target_obj,
            range: range_sq_term,
            occluded: false,
        },
        medium: Medium::Vacuum,
        interference: Interference {
            noise_floor: Medium::Vacuum.attenuation(),
            jamming: 0,
        },
        at: stamp,
    };
    let outcome = resolve_detection(&prop_ctx, scaled_signature);

    SpaceContactObservation {
        ship_id: target.id.clone(),
        callsign: target.callsign.clone(),
        dist_m,
        light_lag_ms: lag_ms,
        ir_signature,
        emcon: target.emcon,
        outcome,
    }
}

/// Pure helper used by [`LocalSpaceBubble::holotank_feed`] and [`LocalSpaceBubble::cockpit_feed`]
/// to filter out undetected contacts (`D-35-10`, `35a` §1).
#[must_use]
pub fn is_bubble_ship_detected(
    observer: &SpaceShipState,
    target: &SpaceShipState,
    at_ic: i64,
) -> bool {
    evaluate_space_contact_detection(observer, ObjId(1), target, ObjId(2), None, at_ic)
        .outcome
        .is_detected()
}

/// Evaluate all other ships in `active.bubble` from the perspective of `hull`'s carrier ship.
#[must_use]
pub fn observe_bubble_contacts(
    active: &ActiveInterdictionBubble,
    hull: ObjId,
    at_ic: i64,
) -> Vec<SpaceContactObservation> {
    let Some(observer) = active.bubble.ships.get(&active.carrier_ship_id) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (ship_id, target) in &active.bubble.ships {
        if ship_id == &active.carrier_ship_id {
            continue;
        }
        let target_obj = if ship_id == &active.interdictor_ship_id {
            active.interdictor_hull
        } else {
            ObjId(hull.0.saturating_add(1_000))
        };
        out.push(evaluate_space_contact_detection(
            observer,
            hull,
            target,
            target_obj,
            Some(active.domain),
            at_ic,
        ));
    }
    out
}

/// Emit viewer-filtered `space.holotank` and `space.cockpit` panel frames to `actor` containing
/// **only** detected contacts (`D-35-10`, `35a` §1).
fn push_space_panels(ctx: &mut Ctx<'_>, hull: ObjId) {
    let Some((holotank_payload, cockpit_payload)) = ({
        let book = ctx.w.module_state::<SpaceBubbleBook>();
        book.and_then(|b| b.active_for_hull(hull)).map(|active| {
            (
                active.bubble.holotank_feed(&active.carrier_ship_id),
                active.bubble.cockpit_feed(&active.carrier_ship_id),
            )
        })
    }) else {
        return;
    };
    ctx.out.push(
        Audience::Actor(ctx.actor),
        OutputEvent::Panel {
            feed: HOLOTANK_FEED.into(),
            kind: PanelFrameKind::Keyframe,
            data: holotank_payload,
        },
    );
    ctx.out.push(
        Audience::Actor(ctx.actor),
        OutputEvent::Panel {
            feed: SPACE_COCKPIT_FEED.into(),
            kind: PanelFrameKind::Keyframe,
            data: cockpit_payload,
        },
    );
}

// ---------------------------------------------------------------------------
// `+helm` handler and switches
// ---------------------------------------------------------------------------

/// Top-level command handler for `+helm` (`SF.R3`, `D-35-9`).
///
/// # Errors
///
/// Returns [`CmdError`] if `[tacsim].enabled` is false, the caller is not aboard a ship,
/// the caller lacks the required bridge seat/channel, or the arguments are malformed.
pub fn helm_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    if ctx.switches.iter().count() > 1 {
        return Err(CmdError::msg("Choose one +helm switch."));
    }
    let switch = ctx.switches.iter().next();
    match switch {
        None | Some("status") => helm_status(ctx),
        Some("station") => helm_station(ctx),
        Some("heading") => helm_heading(ctx),
        Some("thrust") => helm_thrust(ctx),
        Some("emcon") => helm_emcon(ctx),
        Some("roll") => helm_roll(ctx),
        Some("guidance") => helm_guidance(ctx),
        Some("decouple") => helm_decouple(ctx),
        Some("burn") => helm_burn(ctx),
        Some("autopilot") => helm_autopilot(ctx),
        Some(other) => Err(CmdError::msg(format!("Unknown +helm switch: /{other}."))),
    }
}

fn helm_status(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, role) = require_aboard_and_seat(ctx, None)?;
    let hull_name = display_name(ctx.w, hull);
    let mut lines = vec![format!(
        "Helm status aboard {hull_name} (#{}) — your station: `{}` ({}).",
        hull.0,
        role.station_token(),
        role.channel_names().join(", ")
    )];

    if let Some(sim) = ctx.w.module_state::<SpatialState>()
        && let Some(assignments) = sim.assignments.get(&hull)
        && !assignments.seats.is_empty()
    {
        let mut seat_parts = Vec::new();
        for (sid, seat) in &assignments.seats {
            seat_parts.push(format!(
                "{}: {}",
                sid.as_str(),
                display_name(ctx.w, seat.actor)
            ));
        }
        lines.push(format!("  Bridge roster: {}.", seat_parts.join(", ")));
    }

    if let Some(book) = ctx.w.module_state::<SpaceBubbleBook>() {
        let mut order_summaries = Vec::new();
        for channel in OrderChannel::all() {
            if let Some(order) = book.standing_order(hull, channel) {
                order_summaries.push(format!(
                    "{}={} (rev {})",
                    channel.token(),
                    order.value().render(),
                    order.revision()
                ));
            }
        }
        if !order_summaries.is_empty() {
            lines.push(format!(
                "  Standing orders: {}.",
                order_summaries.join("; ")
            ));
        }
        if let Some(active) = book.active_for_hull(hull)
            && let Some(ship) = active.bubble.ships.get(&active.carrier_ship_id)
        {
            let (yaw_deg, pitch_deg, roll_deg) = ship.attitude_deg;
            lines.push(format!(
                "  Bubble #{} — pos ({}, {}, {}) m, vel ({}, {}, {}) mm/s, attitude yaw={}° pitch={}° roll={}°, thrust=({}, {}, {}) m/s², EMCON={}.",
                active.bubble.id.0,
                ship.pos_m[0],
                ship.pos_m[1],
                ship.pos_m[2],
                ship.vel_mmps[0],
                ship.vel_mmps[1],
                ship.vel_mmps[2],
                yaw_deg,
                pitch_deg,
                roll_deg,
                ship.thrust_mps2[0],
                ship.thrust_mps2[1],
                ship.thrust_mps2[2],
                ship.emcon.code_str(),
            ));
        }
        if let Some(ff) = book.free_flight_for_hull(hull).cloned() {
            let burn_rem = ff
                .burn_remaining_ic_s
                .map_or_else(|| "none".to_owned(), |s| format!("{s}s"));
            lines.push(format!(
                "  Free flight ({} -> {}) — pos ({}, {}, {}) km, vel ({}, {}, {}) m/s, thrust ({}, {}, {}) mm/s² (burn remaining: {burn_rem}), guidance={}.",
                ff.origin_place,
                ff.target_place,
                ff.pos_km[0],
                ff.pos_km[1],
                ff.pos_km[2],
                ff.vel_mps[0],
                ff.vel_mps[1],
                ff.vel_mps[2],
                ff.thrust_mmps2[0],
                ff.thrust_mmps2[1],
                ff.thrust_mmps2[2],
                ff.guidance.code_str(),
            ));
            if let Ok((basis, live_kg)) = live_hull_basis_and_propellant_kg(ctx, hull) {
                let non_prop_payload = basis
                    .payload
                    .saturating_sub(basis.tank.saturating_mul(1_000));
                let dv_mps = crate::port::passage::remaining_dv_mps_for_propellant(
                    &basis.spec.drive,
                    basis.spec.dry_kg,
                    non_prop_payload,
                    live_kg,
                )
                .unwrap_or(0);
                let reserve_clause = compute_hull_return_reserve(
                    ctx.w,
                    &basis,
                    live_kg,
                    ff.pos_km,
                    ff.vel_mps,
                    ctx.w.world_ic_now().at,
                )
                .map_or_else(String::new, |(res, port)| {
                    format!(
                        ", RETURN_RESERVE={} kg ({} t, {} m/s Δv to {port})",
                        res.reserve_kg, res.plan.burn_t, res.reserve_dv_mps
                    )
                });
                lines.push(format!(
                    "  Propellant: {live_kg} kg ({:.1} t), remaining Δv={dv_mps} m/s{reserve_clause}.",
                    (live_kg as f64) / 1_000.0
                ));
            }
        }
    }

    for line in lines {
        ctx.out.sys_to_actor(ctx.actor, "tacsim.space", line);
    }
    push_space_panels(ctx, hull);
    Ok(())
}

fn resolve_crew_aboard_hull(
    w: &World,
    actor: ObjId,
    hull: ObjId,
    token: &str,
) -> Result<ObjId, CmdError> {
    let trimmed = token.trim().trim_start_matches('*').trim();
    if let Some(id_str) = trimmed.strip_prefix('#')
        && let Ok(raw_id) = id_str.parse::<u32>()
    {
        let candidate = ObjId(raw_id);
        if hull_aboard_for_actor(w, candidate) == Some(hull) {
            return Ok(candidate);
        }
        return Err(CmdError::msg(
            "That person is not physically aboard this ship.",
        ));
    }

    let mut rooms = BTreeSet::new();
    if let Some(loc) = w.get(actor).and_then(|o| o.location) {
        rooms.insert(loc);
    }
    rooms.insert(hull);
    if let Some(view) = crate::port::ship::hull_view(w, hull)
        && let Some(airlock) = view.airlock()
    {
        rooms.insert(airlock);
    }
    if let Some(sim) = w.module_state::<SpatialState>() {
        for (room_id, space) in &sim.spaces {
            if matches!(space.anchor, mush_sim::SpaceAnchor::MovingObject { host } if host == hull)
            {
                rooms.insert(*room_id);
            }
        }
    }

    let mut candidates = BTreeSet::new();
    for room in rooms {
        if let Some(obj) = w.get(room) {
            for &item in &obj.contents {
                candidates.insert(item);
                if let Some(sub) = w.get(item) {
                    for &nested in &sub.contents {
                        candidates.insert(nested);
                    }
                }
            }
        }
    }

    let lower = trimmed.to_ascii_lowercase();
    for candidate in &candidates {
        if hull_aboard_for_actor(w, *candidate) == Some(hull)
            && w.get(*candidate)
                .is_some_and(|o| o.name.eq_ignore_ascii_case(trimmed))
        {
            return Ok(*candidate);
        }
    }
    for candidate in &candidates {
        if hull_aboard_for_actor(w, *candidate) == Some(hull)
            && w.get(*candidate)
                .is_some_and(|o| o.name.to_ascii_lowercase().starts_with(&lower))
        {
            return Ok(*candidate);
        }
    }

    Err(CmdError::msg(
        "That person is not physically aboard this ship.",
    ))
}

fn helm_station(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let hull =
        hull_aboard_for_actor(ctx.w, ctx.actor).ok_or_else(|| CmdError::msg(NOT_ABOARD_SHIP))?;
    let raw = ctx.args.raw().trim().to_owned();
    if raw.is_empty() {
        return Err(CmdError::msg(
            "Usage: +helm/station <command|helm|tactical|ew|engineering|flight_ops>[=<crew>] | +helm/station off",
        ));
    }

    if raw.eq_ignore_ascii_case("off") || raw.eq_ignore_ascii_case("vacate") {
        let stations_held: Vec<StationId> = ctx
            .w
            .module_state::<SpatialState>()
            .and_then(|sim| sim.assignments.get(&hull))
            .map(|ha| {
                ha.seats
                    .iter()
                    .filter(|(_, s)| s.actor == ctx.actor)
                    .map(|(sid, _)| sid.clone())
                    .collect()
            })
            .unwrap_or_default();
        if stations_held.is_empty() {
            return Err(CmdError::msg(NO_BRIDGE_STATION));
        }
        for sid in &stations_held {
            mush_sim::vacate_station(ctx, hull, sid, ctx.actor)
                .map_err(|e| CmdError::msg(e.to_string()))?;
        }
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.space",
            format!(
                "You vacate your bridge station aboard {}.",
                display_name(ctx.w, hull)
            ),
        );
        return Ok(());
    }

    let (seat_token, target_token) = match raw.split_once('=') {
        Some((lhs, rhs)) => (lhs.trim(), Some(rhs.trim())),
        None => (raw.as_str(), None),
    };
    let role = BridgeRole::parse(seat_token).ok_or_else(|| {
        CmdError::msg(
            "Unknown bridge station (choose command, helm, tactical, ew, engineering, or flight_ops).",
        )
    })?;

    // Only the ship's captain or current `command` station holder may assign bridge stations.
    let caller_role = seat_role_for_actor(ctx.w, hull, ctx.actor);
    if caller_role != Some(BridgeRole::Command) && !is_hull_captain(ctx.w, hull, ctx.actor) {
        return Err(CmdError::msg(
            "Only the ship's captain or command station may assign bridge stations.",
        ));
    }

    let assignee = match target_token {
        None | Some("me") | Some("self") => ctx.actor,
        Some(who) => resolve_crew_aboard_hull(ctx.w, ctx.actor, hull, who)?,
    };

    let station_id = StationId::new(role.station_token())
        .ok_or_else(|| CmdError::msg("Invalid station identifier."))?;
    let channels = role.channel_set();
    let bridge_room = ctx.w.get(ctx.actor).and_then(|o| o.location);
    mush_sim::declare_station(
        ctx,
        hull,
        &station_id,
        role.role_name(),
        &channels,
        bridge_room,
        role.priority(),
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;

    // Vacate any previous station held by `assignee` on `hull` and any prior occupant of `station_id`
    // so the captain can reassign stations cleanly.
    let existing_holder = ctx
        .w
        .module_state::<SpatialState>()
        .and_then(|sim| sim.assignments.get(&hull))
        .and_then(|ha| ha.seats.get(&station_id))
        .map(|s| s.actor);
    if let Some(prev) = existing_holder
        && prev != assignee
    {
        mush_sim::vacate_station(ctx, hull, &station_id, prev)
            .map_err(|e| CmdError::msg(e.to_string()))?;
    }
    let previous_seats: Vec<StationId> = ctx
        .w
        .module_state::<SpatialState>()
        .and_then(|sim| sim.assignments.get(&hull))
        .map(|ha| {
            ha.seats
                .iter()
                .filter(|(sid, s)| s.actor == assignee && *sid != &station_id)
                .map(|(sid, _)| sid.clone())
                .collect()
        })
        .unwrap_or_default();
    for prev_sid in previous_seats {
        mush_sim::vacate_station(ctx, hull, &prev_sid, assignee)
            .map_err(|e| CmdError::msg(e.to_string()))?;
    }

    mush_sim::commands::assign_station(ctx, hull, &station_id, assignee)?;
    Ok(())
}

fn helm_heading(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("movement"))?;
    let raw = ctx.args.raw().trim();
    if raw.is_empty() {
        return Err(CmdError::msg(
            "Usage: +helm/heading <yaw_deg>[,<pitch_deg>]",
        ));
    }
    let parts: Vec<&str> = raw
        .split([',', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let yaw_deg: i32 = parts
        .first()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| CmdError::msg("Yaw must be an integer in degrees (0..359)."))?;
    let pitch_deg: i32 = match parts.get(1) {
        Some(s) => s
            .parse()
            .map_err(|_| CmdError::msg("Pitch must be an integer in degrees (-90..90)."))?,
        None => 0,
    };
    if !(0..360).contains(&yaw_deg) {
        return Err(CmdError::msg("Yaw must be in 0..359 degrees."));
    }
    if !(-90..=90).contains(&pitch_deg) {
        return Err(CmdError::msg("Pitch must be in -90..=90 degrees."));
    }

    let (domain, generation, tick) = domain_stamp_for_hull(ctx.w, hull);
    let vector = heading_order_vector(yaw_deg, pitch_deg);
    let order = record_hull_standing_order(
        ctx.w,
        hull,
        OrderChannel::Heading,
        vector,
        ctx.actor,
        domain,
        generation,
        tick,
    )?;

    if let Some((ship_id, roll_deg)) = active_carrier_ship_and_roll(ctx.w, hull) {
        let _ = apply_bubble_command(
            ctx.w,
            hull,
            BubbleInputCommand::SetAttitude {
                ship_id,
                attitude_deg: (yaw_deg, pitch_deg, roll_deg),
            },
        );
    }

    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "Helm heading set: yaw {yaw_deg}°, pitch {pitch_deg}° (standing order `{}` rev {}).",
            order.channel().token(),
            order.revision()
        ),
    );
    push_space_panels(ctx, hull);
    Ok(())
}

fn helm_thrust(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("movement"))?;
    let raw = ctx.args.raw().trim();
    if raw.is_empty() {
        return Err(CmdError::msg(
            "Usage: +helm/thrust <permille 0..1000>[,<dir_x>,<dir_y>,<dir_z>] [force]",
        ));
    }
    let all_parts: Vec<&str> = raw
        .split([',', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let force_into_reserve = all_parts.iter().any(|t| t.eq_ignore_ascii_case("force"));
    let parts: Vec<&str> = all_parts
        .into_iter()
        .filter(|t| !t.eq_ignore_ascii_case("force"))
        .collect();
    let thrust_permille: u16 = parts
        .first()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| CmdError::msg("Thrust must be 0..=1000 permille."))?;
    if thrust_permille > 1_000 {
        return Err(CmdError::msg("Thrust must be 0..=1000 permille."));
    }

    let burn_dir = if parts.len() >= 4 {
        let dx: i32 = parts[1].parse().map_err(|_| {
            CmdError::msg("Burn vector components must be integers (-1000..=1000).")
        })?;
        let dy: i32 = parts[2].parse().map_err(|_| {
            CmdError::msg("Burn vector components must be integers (-1000..=1000).")
        })?;
        let dz: i32 = parts[3].parse().map_err(|_| {
            CmdError::msg("Burn vector components must be integers (-1000..=1000).")
        })?;
        if !(-1_000..=1_000).contains(&dx)
            || !(-1_000..=1_000).contains(&dy)
            || !(-1_000..=1_000).contains(&dz)
        {
            return Err(CmdError::msg(
                "Burn vector components must be in -1000..=1000.",
            ));
        }
        [dx, dy, dz]
    } else {
        default_nose_burn_dir(ctx.w, hull)
    };

    let (domain, generation, tick) = domain_stamp_for_hull(ctx.w, hull);
    let order = record_hull_standing_order(
        ctx.w,
        hull,
        OrderChannel::Thrust,
        OrderValue::Scalar {
            value: i32::from(thrust_permille),
        },
        ctx.actor,
        domain,
        generation,
        tick,
    )?;

    if let Some((ship_id, max_accel_mps2)) = active_carrier_ship_and_max_accel(ctx.w, hull) {
        let scale = i64::from(thrust_permille);
        let thrust_mps2 = if scale == 0 {
            [0, 0, 0]
        } else {
            let ax = (max_accel_mps2 * scale * i64::from(burn_dir[0])) / 1_000_000;
            let ay = (max_accel_mps2 * scale * i64::from(burn_dir[1])) / 1_000_000;
            let az = (max_accel_mps2 * scale * i64::from(burn_dir[2])) / 1_000_000;
            if ax == 0 && ay == 0 && az == 0 {
                [
                    i64::from(burn_dir[0].signum()),
                    i64::from(burn_dir[1].signum()),
                    i64::from(burn_dir[2].signum()),
                ]
            } else {
                [ax, ay, az]
            }
        };
        let _ = apply_bubble_command(
            ctx.w,
            hull,
            BubbleInputCommand::CommandBurn {
                ship_id,
                thrust_mps2,
                force_into_reserve,
            },
        );
    }

    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "Helm thrust set: {thrust_permille}‰ burn=({}, {}, {}) (standing order `{}` rev {}).",
            burn_dir[0],
            burn_dir[1],
            burn_dir[2],
            order.channel().token(),
            order.revision()
        ),
    );
    push_space_panels(ctx, hull);
    Ok(())
}

fn helm_emcon(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("sensors"))?;
    let raw = ctx.args.raw().trim();
    let (level, posture) = parse_emcon_token(raw)
        .ok_or_else(|| CmdError::msg("Usage: +helm/emcon <ghost|passive|standard|combat>"))?;

    let (domain, generation, tick) = domain_stamp_for_hull(ctx.w, hull);
    let order = record_hull_standing_order(
        ctx.w,
        hull,
        OrderChannel::Posture,
        OrderValue::Emcon { level },
        ctx.actor,
        domain,
        generation,
        tick,
    )?;

    if let Some((ship_id, _)) = active_carrier_ship_and_roll(ctx.w, hull) {
        let _ = apply_bubble_command(
            ctx.w,
            hull,
            BubbleInputCommand::SetEmcon { ship_id, posture },
        );
    }

    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "EMCON posture set: {} (level {level}, standing order `{}` rev {}).",
            posture.code_str(),
            order.channel().token(),
            order.revision()
        ),
    );
    push_space_panels(ctx, hull);
    Ok(())
}

fn helm_roll(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("movement"))?;
    let roll_deg: i32 = ctx
        .args
        .raw()
        .trim()
        .parse()
        .map_err(|_| CmdError::msg("Usage: +helm/roll <roll_deg -180..180>"))?;
    if !(-180..=180).contains(&roll_deg) {
        return Err(CmdError::msg("Roll must be in -180..=180 degrees."));
    }

    if let Some((ship_id, yaw_deg, pitch_deg)) = active_carrier_attitude(ctx.w, hull) {
        let _ = apply_bubble_command(
            ctx.w,
            hull,
            BubbleInputCommand::SetAttitude {
                ship_id,
                attitude_deg: (yaw_deg, pitch_deg, roll_deg),
            },
        );
    }

    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!("Helm roll set: {roll_deg}°."),
    );
    push_space_panels(ctx, hull);
    Ok(())
}

fn helm_guidance(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("movement"))?;
    let raw = ctx.args.raw().trim().to_owned();
    if let Some(mode) = FreeFlightGuidanceMode::parse(&raw) {
        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
            && let Some(ff) = book.free_flight_for_hull_mut(hull)
        {
            ff.guidance = mode;
        }
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.space",
            format!("Helm guidance mode set: `{}`.", mode.code_str()),
        );
        push_space_panels(ctx, hull);
        return Ok(());
    }
    let yaw_deg: i32 = raw.parse().map_err(|_| {
        CmdError::msg(
            "Usage: +helm/guidance <attitude_deg 0..359 | manual|prograde|retrograde|radial_in|radial_out|normal|anti_normal|intercept|match>",
        )
    })?;
    if !(0..360).contains(&yaw_deg) {
        return Err(CmdError::msg(
            "Guidance attitude must be in 0..359 degrees.",
        ));
    }

    if let Some((ship_id, _, pitch_deg)) = active_carrier_attitude(ctx.w, hull) {
        let roll_deg = active_carrier_ship_and_roll(ctx.w, hull).map_or(0, |(_, r)| r);
        let _ = apply_bubble_command(
            ctx.w,
            hull,
            BubbleInputCommand::SetAttitude {
                ship_id,
                attitude_deg: (yaw_deg, pitch_deg, roll_deg),
            },
        );
    }

    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!("Helm guidance attitude set: {yaw_deg}°."),
    );
    push_space_panels(ctx, hull);
    Ok(())
}

// ---------------------------------------------------------------------------
// Free-flight decoupling, live propellant burns, and passage planner (`SF.R4`)
// ---------------------------------------------------------------------------

fn live_hull_basis_and_propellant_kg(
    ctx: &Ctx<'_>,
    hull: ObjId,
) -> Result<(crate::port::voyage::Basis, u64), CmdError> {
    let view = crate::port::ship::hull_view(ctx.w, hull)
        .ok_or_else(|| CmdError::msg("Ship has no registered hull view."))?;
    let souls = view
        .airlock()
        .map_or(1, |a| crate::port::voyage::occupants(ctx.w, a).max(1));
    let basis = crate::port::voyage::basis(ctx, &view, souls).map_err(CmdError::msg)?;
    let live_kg = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| b.propellant_ledger(hull))
        .map_or_else(
            || basis.tank.saturating_mul(1_000),
            |l| l.effective_live_kg(basis.tank),
        );
    Ok((basis, live_kg))
}

fn open_port_places(
    w: &World,
) -> Vec<(
    mush_core::CanonicalId,
    String,
    ObjId,
    crate::port::def::PortDef,
)> {
    let mut out = Vec::new();
    for (def, map) in crate::port::voyage::lane_ports(w) {
        if def.site.is_some() {
            continue;
        }
        let Some(map) = map else {
            continue;
        };
        let Some(host_raw) = map.host else {
            continue;
        };
        let host = ObjId(host_raw);
        let Some(place_id) =
            crate::port::voyage::place_id_of(w, host).or_else(|| match def.venue.as_str() {
                "ceres_freeport" => mush_core::CanonicalId::new("ceres_orbit").ok(),
                "stickney_yards" => mush_core::CanonicalId::new("stickney_berths").ok(),
                _ => None,
            })
        else {
            continue;
        };
        out.push((
            place_id,
            crate::port::coop::port_name(def),
            host,
            def.clone(),
        ));
    }
    out
}

fn compute_hull_return_reserve(
    w: &World,
    basis: &crate::port::voyage::Basis,
    live_kg: u64,
    pos_km: [i64; 3],
    vel_mps: [i64; 3],
    at_ic: i64,
) -> Option<(crate::port::passage::ReturnReserve, String)> {
    let (sky, pack) = crate::port::voyage::lane_sky_of(w, basis.version).ok()?;
    let open_ports = open_port_places(w);
    let place_ids: Vec<mush_core::CanonicalId> = open_ports
        .iter()
        .map(|(pid, _, _, _)| pid.clone())
        .collect();
    if place_ids.is_empty() {
        return None;
    }
    let non_prop_payload = basis
        .payload
        .saturating_sub(basis.tank.saturating_mul(1_000));
    let effective_payload = non_prop_payload.saturating_add(live_kg);
    let sky_pos = crate::sky::PosKm {
        x: pos_km[0],
        y: pos_km[1],
        z: pos_km[2],
    };
    let sky_vel = [
        i32::try_from(vel_mps[0].clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0),
        i32::try_from(vel_mps[1].clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0),
        i32::try_from(vel_mps[2].clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0),
    ];
    let reserve = crate::port::passage::return_reserve_from(
        &sky,
        pack,
        sky_pos,
        sky_vel,
        &place_ids,
        &basis.spec.drive,
        basis.spec.dry_kg,
        effective_payload,
        basis.occupants,
        at_ic,
    )
    .ok()?;
    let label = open_ports
        .iter()
        .find(|(pid, _, _, _)| *pid == reserve.port)
        .map_or_else(|| reserve.port.to_string(), |(_, name, _, _)| name.clone());
    Some((reserve, label))
}

fn submit_space_burn_if_needed(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    vault: u64,
    burn_t: u64,
    crew_days: u32,
) -> Result<Option<u32>, CmdError> {
    if burn_t == 0 && crew_days == 0 {
        return Ok(None);
    }
    let burn_handle = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .and_then(|h| h.burn.clone())
        .ok_or_else(|| CmdError::msg("The port burn action is not wired."))?;
    let seq = {
        let book = ctx
            .w
            .module_state_mut::<SpaceBubbleBook>()
            .ok_or_else(|| CmdError::msg("Space bubble state is not registered."))?;
        let seq = book.alloc_space_burn_seq();
        book.pending_space_burns
            .push(crate::port::burn::PendingSpaceBurn {
                seq,
                hull,
                actor: ctx.actor,
                vault,
                burn_t,
                crew_days,
            });
        seq
    };
    let key = mush_core::IdempotencyKeyText::parse_canonical(format!(
        "{}{}/space_burn/{seq}",
        crate::port::burn::BURN_KEY_PREFIX,
        hull.0
    ))
    .map_err(|_| CmdError::msg("Failed to form space burn idempotency key."))?;
    ctx.submit_authoritative(
        burn_handle,
        mush_core::PerceivedRef::SelfActor,
        Vec::new(),
        serde_json::json!({ "seq": seq.to_string() }),
        key,
    );
    Ok(Some(seq))
}

fn helm_decouple(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("movement"))?;
    if ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .is_some_and(|b| b.active_for_hull(hull).is_some())
    {
        return Err(CmdError::msg(
            "Cannot decouple from transit while in an active interdiction bubble.",
        ));
    }
    if ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .is_some_and(|b| b.free_flight_for_hull(hull).is_some())
    {
        return Err(CmdError::msg("Ship is already decoupled in free flight."));
    }

    let journey = ctx
        .w
        .module_state::<JourneyBook>()
        .and_then(|jb| jb.get(hull).cloned())
        .filter(|j| j.state().is_under_way())
        .ok_or_else(|| CmdError::msg("Ship is not under way on a transit corridor."))?;

    let now = ctx.w.world_ic_now();
    let (basis, _) = live_hull_basis_and_propellant_kg(ctx, hull)?;
    let (sky, pack) = crate::port::voyage::lane_sky_of(ctx.w, basis.version)
        .map_err(|e| CmdError::msg(e.to_string()))?;

    let legs = journey.route().legs();
    let cursor = usize::try_from(journey.cursor()).unwrap_or(0);
    let leg_idx = cursor.min(legs.len().saturating_sub(1));
    let from_obj = if leg_idx == 0 {
        journey.route().endpoints().0
    } else {
        legs.get(leg_idx.saturating_sub(1))
            .map_or(journey.route().endpoints().0, |l| l.to())
    };
    let to_obj = legs
        .get(leg_idx)
        .map_or(journey.route().endpoints().1, |l| l.to());

    let origin_place = crate::port::voyage::place_id_of(ctx.w, from_obj)
        .or_else(|| mush_core::CanonicalId::new("ceres_orbit").ok())
        .ok_or_else(|| CmdError::msg("Could not resolve leg origin place."))?;
    let target_place = crate::port::voyage::place_id_of(ctx.w, to_obj)
        .or_else(|| mush_core::CanonicalId::new("stickney_berths").ok())
        .ok_or_else(|| CmdError::msg("Could not resolve leg target place."))?;

    let origin_body = crate::port::passage::body_or_place_of(&sky, pack, &origin_place)
        .unwrap_or_else(|_| origin_place.clone());
    let target_body = crate::port::passage::body_or_place_of(&sky, pack, &target_place)
        .unwrap_or_else(|_| target_place.clone());
    let p0 = sky
        .position_km(&origin_body, now.at)
        .unwrap_or(crate::sky::PosKm {
            x: 414_000_000,
            y: 0,
            z: 0,
        });
    let p1 = sky
        .position_km(&target_body, now.at)
        .unwrap_or(crate::sky::PosKm {
            x: 227_940_000,
            y: 0,
            z: 0,
        });
    let [vx0, vy0, vz0] = sky
        .velocity_mps(&origin_body, now.at)
        .unwrap_or([0, 17_900, 0]);

    let leg_s = i64::from(
        journey
            .leg_secs(leg_idx)
            .or_else(|| legs.get(leg_idx).map(|l| l.base_s()))
            .unwrap_or(86_400)
            .max(1),
    );
    let leg_start_ic = if leg_idx == 0 {
        journey.departed().at
    } else {
        journey
            .leg_due_at(leg_idx.saturating_sub(1))
            .unwrap_or(journey.departed().at)
    };
    let elapsed_s = now.at.saturating_sub(leg_start_ic).max(0);
    let frac = if elapsed_s == 0 {
        0.15
    } else {
        ((elapsed_s as f64) / (leg_s as f64)).clamp(0.05, 0.95)
    };
    let pos_km = [
        p0.x + (((p1.x - p0.x) as f64) * frac).round() as i64,
        p0.y + (((p1.y - p0.y) as f64) * frac).round() as i64,
        p0.z + (((p1.z - p0.z) as f64) * frac).round() as i64,
    ];
    let chord_vel_mps = [
        ((i128::from(p1.x) - i128::from(p0.x)) * 1_000 / i128::from(leg_s)) as i64,
        ((i128::from(p1.y) - i128::from(p0.y)) * 1_000 / i128::from(leg_s)) as i64,
        ((i128::from(p1.z) - i128::from(p0.z)) * 1_000 / i128::from(leg_s)) as i64,
    ];
    let vel_mps = [
        i64::from(vx0).saturating_add(chord_vel_mps[0] / 4),
        i64::from(vy0).saturating_add(chord_vel_mps[1] / 4),
        i64::from(vz0).saturating_add(chord_vel_mps[2] / 4),
    ];

    let domain = ctx
        .w
        .module_state::<SpatialState>()
        .and_then(|s| {
            s.domain_of_host(to_obj)
                .or_else(|| s.domain_of_host(from_obj))
        })
        .map_or(DomainId(1), |d| d.id);
    let _ = mush_sim::promote_carrier_journey(ctx, hull, domain, &now);

    let record = SpaceFreeFlightRecord {
        hull,
        origin_place: origin_place.clone(),
        target_place: target_place.clone(),
        pos_km,
        vel_mps,
        thrust_mmps2: [0, 0, 0],
        burn_remaining_ic_s: None,
        guidance: FreeFlightGuidanceMode::Manual,
        braking_flipped: false,
        decoupled_at_ic_s: now.at,
        last_ic_s: now.at,
    };
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        book.free_flights.insert(hull, record);
    }

    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "Decoupled from corridor leg `{origin_place} -> {target_place}` into 3D heliocentric free flight at pos=({}, {}, {}) km, vel=({}, {}, {}) m/s.",
            pos_km[0], pos_km[1], pos_km[2], vel_mps[0], vel_mps[1], vel_mps[2]
        ),
    );
    Ok(())
}

fn helm_burn(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("movement"))?;
    let raw = ctx.args.raw().trim();
    if raw.is_empty() {
        return Err(CmdError::msg(
            "Usage: +helm/burn <ax_mps2>,<ay_mps2>,<az_mps2> [<dur_ic_s>] [force]",
        ));
    }

    let tokens: Vec<&str> = raw.split_whitespace().collect();
    let force_into_reserve = tokens.iter().any(|t| t.eq_ignore_ascii_case("force"));
    let non_force: Vec<&str> = tokens
        .into_iter()
        .filter(|t| !t.eq_ignore_ascii_case("force"))
        .collect();
    let vec_token = non_force.first().ok_or_else(|| {
        CmdError::msg("Usage: +helm/burn <ax_mps2>,<ay_mps2>,<az_mps2> [<dur_ic_s>] [force]")
    })?;
    let axes: Vec<&str> = vec_token.split(',').map(str::trim).collect();
    if axes.len() != 3 {
        return Err(CmdError::msg(
            "Burn acceleration must be three comma-separated integers: <ax>,<ay>,<az> (m/s²).",
        ));
    }
    let ax_mps2: i64 = axes[0]
        .parse()
        .map_err(|_| CmdError::msg("Burn ax must be an integer (m/s²)."))?;
    let ay_mps2: i64 = axes[1]
        .parse()
        .map_err(|_| CmdError::msg("Burn ay must be an integer (m/s²)."))?;
    let az_mps2: i64 = axes[2]
        .parse()
        .map_err(|_| CmdError::msg("Burn az must be an integer (m/s²)."))?;
    if ax_mps2.abs() > 100 || ay_mps2.abs() > 100 || az_mps2.abs() > 100 {
        return Err(CmdError::msg(
            "Burn acceleration components must be in -100..=100 m/s².",
        ));
    }

    let dur_ic_s: u32 = match non_force.get(1) {
        Some(s) => s
            .parse::<u32>()
            .map_err(|_| CmdError::msg("Burn duration must be a positive integer (IC seconds)."))?,
        None => 60,
    };
    if dur_ic_s == 0 || dur_ic_s > 86_400 {
        return Err(CmdError::msg(
            "Burn duration must be in 1..=86400 IC seconds.",
        ));
    }

    let ff = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| b.free_flight_for_hull(hull).cloned())
        .ok_or_else(|| {
            CmdError::msg("Ship is not in free flight; use +helm/decouple first (or +helm/thrust inside a bubble).")
        })?;

    let accel_sq =
        i128::from(ax_mps2).pow(2) + i128::from(ay_mps2).pow(2) + i128::from(az_mps2).pow(2);
    let accel_mag_mps2 = libm::sqrt(accel_sq as f64);
    let dv_mps = (accel_mag_mps2 * (dur_ic_s as f64)).round() as u32;

    let (basis, live_kg) = live_hull_basis_and_propellant_kg(ctx, hull)?;
    let non_prop_payload = basis
        .payload
        .saturating_sub(basis.tank.saturating_mul(1_000));
    let effective_payload = non_prop_payload.saturating_add(live_kg);
    let burn_kg = crate::port::passage::burn_kg_for_dv(
        &basis.spec.drive,
        basis.spec.dry_kg,
        effective_payload,
        dv_mps,
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;

    if burn_kg > live_kg {
        return Err(CmdError::msg(format!(
            "Insufficient propellant: burn requires {burn_kg} kg ({dv_mps} m/s Δv), tank holds {live_kg} kg."
        )));
    }

    let now_ic = ctx.w.world_ic_now().at;
    if let Some((reserve, nearest_port)) =
        compute_hull_return_reserve(ctx.w, &basis, live_kg, ff.pos_km, ff.vel_mps, now_ic)
    {
        let remaining_after_burn_kg = live_kg.saturating_sub(burn_kg);
        if remaining_after_burn_kg < reserve.reserve_kg && !force_into_reserve {
            return Err(CmdError::msg(format!(
                "RETURN_RESERVE warning: burn requires {burn_kg} kg ({dv_mps} m/s Δv), leaving {remaining_after_burn_kg} kg below the {} kg ({} t, {} m/s Δv) return reserve to {nearest_port}. Pass `force` to override.",
                reserve.reserve_kg, reserve.plan.burn_t, reserve.reserve_dv_mps
            )));
        }
    }

    let whole_tonnes = {
        let book = ctx
            .w
            .module_state_mut::<SpaceBubbleBook>()
            .ok_or_else(|| CmdError::msg("Space bubble state is not registered."))?;
        let ledger = book.propellant_ledger_mut(hull);
        let tonnes = ledger.record_burn_kg(basis.tank, burn_kg);
        if let Some(rec) = book.free_flight_for_hull_mut(hull) {
            rec.thrust_mmps2 = [ax_mps2 * 1_000, ay_mps2 * 1_000, az_mps2 * 1_000];
            rec.burn_remaining_ic_s = Some(dur_ic_s);
        }
        tonnes
    };

    let _ = submit_space_burn_if_needed(ctx, hull, basis.vault.0, whole_tonnes, 0)?;

    let (domain, generation, tick) = domain_stamp_for_hull(ctx.w, hull);
    let throttle_scalar = ((accel_mag_mps2 * 200.0).round() as i32).clamp(0, MAX_ORDER_SCALAR);
    let order = record_hull_standing_order(
        ctx.w,
        hull,
        OrderChannel::Thrust,
        OrderValue::Scalar {
            value: throttle_scalar,
        },
        ctx.actor,
        domain,
        generation,
        tick,
    )?;

    let after_kg = live_kg.saturating_sub(burn_kg);
    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "Timed burn engaged: ({ax_mps2}, {ay_mps2}, {az_mps2}) m/s² for {dur_ic_s}s (Δv={dv_mps} m/s, burned {burn_kg} kg, {after_kg} kg remaining; standing order `{}` rev {}).",
            order.channel().token(),
            order.revision()
        ),
    );
    Ok(())
}

fn helm_autopilot(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("movement"))?;
    let port_word = ctx.args.raw().trim().to_owned();
    if port_word.is_empty() {
        return Err(CmdError::msg("Usage: +helm/autopilot <port>"));
    }
    if ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .is_some_and(|b| b.active_for_hull(hull).is_some())
    {
        return Err(CmdError::msg(
            "Cannot engage autopilot while in an active interdiction bubble.",
        ));
    }
    let ff = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| b.free_flight_for_hull(hull).cloned())
        .ok_or_else(|| {
            CmdError::msg(
                "Ship is not in free flight; use +helm/decouple first or +ship/undock from a berth.",
            )
        })?;

    let open_ports = open_port_places(ctx.w);
    let lower = port_word.to_ascii_lowercase();
    let (target_place, port_label, dest_host, dest_def) = open_ports
        .into_iter()
        .find(|(place_id, label, _, def)| {
            place_id.as_str().eq_ignore_ascii_case(&lower)
                || def.venue.as_str().eq_ignore_ascii_case(&lower)
                || label.to_ascii_lowercase().starts_with(&lower)
        })
        .ok_or_else(|| {
            CmdError::msg(format!(
                "Unknown or closed autopilot destination: `{port_word}`."
            ))
        })?;

    let now = ctx.w.world_ic_now();
    let (basis, live_kg) = live_hull_basis_and_propellant_kg(ctx, hull)?;
    let (sky, pack) = crate::port::voyage::lane_sky_of(ctx.w, basis.version)
        .map_err(|e| CmdError::msg(e.to_string()))?;

    let non_prop_payload = basis
        .payload
        .saturating_sub(basis.tank.saturating_mul(1_000));
    let effective_payload = non_prop_payload.saturating_add(live_kg);
    let ff_pos = crate::sky::PosKm {
        x: ff.pos_km[0],
        y: ff.pos_km[1],
        z: ff.pos_km[2],
    };
    let ff_vel = [
        i32::try_from(ff.vel_mps[0].clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0),
        i32::try_from(ff.vel_mps[1].clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0),
        i32::try_from(ff.vel_mps[2].clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0),
    ];
    let plan = crate::port::passage::plan_passage_from(
        &sky,
        pack,
        ff_pos,
        ff_vel,
        &target_place,
        &basis.spec.drive,
        basis.spec.dry_kg,
        effective_payload,
        basis.occupants,
        now.at,
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;

    let live_tank_t = live_kg / 1_000;
    if live_tank_t < plan.burn_t {
        return Err(CmdError::msg(format!(
            "Insufficient propellant for autopilot passage to {port_label}: requires {} t, tank holds {live_tank_t} t.",
            plan.burn_t
        )));
    }
    if basis.locker < u64::from(plan.crew_days) {
        return Err(CmdError::msg(format!(
            "Insufficient provisions for autopilot passage to {port_label}: requires {} crew-days, locker holds {}.",
            plan.crew_days, basis.locker
        )));
    }

    let dest_domain = ctx
        .w
        .module_state::<SpatialState>()
        .and_then(|s| s.domain_of_host(dest_host))
        .map(|d| d.id)
        .ok_or_else(|| CmdError::msg("Destination port has no active domain."))?;
    let pose_val = mush_sim::passive_rules_value(serde_json::json!({}))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    let dest_parent = mush_sim::SpatialParent::Direct {
        domain: dest_domain,
        pose: mush_sim::Pose::passive(pose_val),
    };

    let origin_host = ctx
        .w
        .module_state::<JourneyBook>()
        .and_then(|jb| jb.get(hull).map(|j| j.route().endpoints().0))
        .unwrap_or(dest_host);
    let leg_secs = plan.legs.first().map_or(3_600, |l| l.secs.max(1));
    let leg_def =
        mush_sim::LegDef::new(dest_host, leg_secs).map_err(|e| CmdError::msg(e.to_string()))?;
    let drive_class =
        mush_core::CanonicalId::new("fusion_torch").map_err(|e| CmdError::msg(e.to_string()))?;
    let route_def = mush_sim::RouteDef::new(
        plan.route.clone(),
        (origin_host, dest_host),
        vec![leg_def],
        drive_class,
        plan.burn_t.max(1),
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;

    mush_sim::recouple_carrier_journey(ctx, hull, route_def, dest_parent, vec![leg_secs], &now)
        .map_err(|e| CmdError::msg(e.to_string()))?;

    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        book.free_flights.remove(&hull);
        book.propellant_ledger_mut(hull)
            .record_passage_burn_t(basis.tank, plan.burn_t);
    }
    let _ = submit_space_burn_if_needed(ctx, hull, basis.vault.0, plan.burn_t, plan.crew_days)?;

    if let Some(mut voyage) =
        ctx.w
            .module_state::<crate::store::TacsimStore>()
            .and_then(|s| match s.voyage_state(hull) {
                crate::port::voyage::VoyageState::Present(v) => Some(v.clone()),
                crate::port::voyage::VoyageState::Absent
                | crate::port::voyage::VoyageState::Unreadable => None,
            })
    {
        if let Some(dep) = &mut voyage.departed {
            dep.to_venue = dest_def.venue.clone();
            dep.depart_ic = now.at;
        }
        crate::store::put_voyage(ctx, &voyage);
    }

    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "Autopilot engaged for {port_label} (`{target_place}`): burn {} t reaction mass, {} crew-days, transit {}s ({} IC days).",
            plan.burn_t,
            plan.crew_days,
            plan.total_s,
            crate::port::voyage::ic_days(plan.total_s)
        ),
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_hull_standing_order(
    w: &mut World,
    hull: ObjId,
    channel: OrderChannel,
    value: OrderValue,
    by: ObjId,
    domain: DomainId,
    generation: u64,
    tick: i64,
) -> Result<StandingOrder, CmdError> {
    let book = w
        .module_state_mut::<SpaceBubbleBook>()
        .ok_or_else(|| CmdError::msg("Space bubble state is not registered."))?;
    book.record_standing_order(hull, channel, value, by, domain, generation, tick)
        .map_err(|e| CmdError::msg(e.to_string()))
}

fn active_carrier_ship_and_roll(w: &World, hull: ObjId) -> Option<(String, i32)> {
    let active = w.module_state::<SpaceBubbleBook>()?.active_for_hull(hull)?;
    let ship = active.bubble.ships.get(&active.carrier_ship_id)?;
    Some((active.carrier_ship_id.clone(), ship.attitude_deg.2))
}

fn active_carrier_ship_and_max_accel(w: &World, hull: ObjId) -> Option<(String, i64)> {
    let active = w.module_state::<SpaceBubbleBook>()?.active_for_hull(hull)?;
    let ship = active.bubble.ships.get(&active.carrier_ship_id)?;
    Some((active.carrier_ship_id.clone(), ship.max_accel_mps2.max(1)))
}

fn active_carrier_attitude(w: &World, hull: ObjId) -> Option<(String, i32, i32)> {
    let active = w.module_state::<SpaceBubbleBook>()?.active_for_hull(hull)?;
    let ship = active.bubble.ships.get(&active.carrier_ship_id)?;
    Some((
        active.carrier_ship_id.clone(),
        ship.attitude_deg.0,
        ship.attitude_deg.1,
    ))
}

fn default_nose_burn_dir(w: &World, hull: ObjId) -> [i32; 3] {
    if let Some((_, yaw_deg, pitch_deg)) = active_carrier_attitude(w, hull)
        && let OrderValue::Vector { x, y, z } = heading_order_vector(yaw_deg, pitch_deg)
    {
        return [x, y, z];
    }
    if let Some(book) = w.module_state::<SpaceBubbleBook>()
        && let Some(order) = book.standing_order(hull, OrderChannel::Heading)
        && let OrderValue::Vector { x, y, z } = *order.value()
    {
        return [x, y, z];
    }
    [1_000, 0, 0]
}

// ---------------------------------------------------------------------------
// `+tac` handler and switches
// ---------------------------------------------------------------------------

/// Top-level command handler for `+tac` (`SF.R3`, `D-35-9`, `D-35-10`).
///
/// # Errors
///
/// Returns [`CmdError`] if `[tacsim].enabled` is false, the caller is not aboard a ship,
/// the caller lacks the required bridge seat/channel, or the target contact is undetected.
pub fn tac_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    if ctx.switches.iter().count() > 1 {
        return Err(CmdError::msg("Choose one +tac switch."));
    }
    let switch = ctx.switches.iter().next();
    match switch {
        None | Some("status") => tac_status(ctx),
        Some("scan") => tac_scan(ctx),
        Some("lock") => tac_lock(ctx),
        Some("beam") => tac_beam(ctx),
        Some("torp") => tac_torp(ctx),
        Some("railgun") => tac_railgun(ctx),
        Some("pdc") => tac_pdc(ctx),
        Some(other) => Err(CmdError::msg(format!("Unknown +tac switch: /{other}."))),
    }
}

fn tac_status(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, role) = require_aboard_and_seat(ctx, None)?;
    if !role.grants_channel("sensors") && !role.grants_channel("fire") {
        return Err(CmdError::msg(format!(
            "Your bridge station ({}) does not control the sensors or fire channel.",
            role.station_token()
        )));
    }
    let Some(book) = ctx.w.module_state::<SpaceBubbleBook>() else {
        return Err(CmdError::msg("No active space bubble."));
    };
    let Some(active) = book.active_for_hull(hull) else {
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.space",
            format!(
                "Tactical status aboard {}: no active space bubble.",
                display_name(ctx.w, hull)
            ),
        );
        return Ok(());
    };
    let Some(ship) = active.bubble.ships.get(&active.carrier_ship_id) else {
        return Err(CmdError::msg("Carrier ship state missing from bubble."));
    };
    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "Tactical status aboard {} ({}): lock={}, PDC auto={}, lasers={}, railguns={}, torpedo tubes={}, heat={}‰/1000‰.",
            display_name(ctx.w, hull),
            ship.id,
            ship.locked_target_id.as_deref().unwrap_or("none"),
            if ship.pdc_auto { "on" } else { "off" },
            ship.lasers.len(),
            ship.railguns.len(),
            ship.torpedo_tubes.len(),
            ship.heat_permille,
        ),
    );
    push_space_panels(ctx, hull);
    Ok(())
}

fn tac_scan(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("sensors"))?;
    let at_ic = ctx.w.world_ic_now().at;
    let lines = {
        let Some(book) = ctx.w.module_state::<SpaceBubbleBook>() else {
            return Err(CmdError::msg("No active space bubble."));
        };
        let Some(active) = book.active_for_hull(hull) else {
            ctx.out.sys_to_actor(
                ctx.actor,
                "tacsim.space",
                "Sensor pass: clear sky (no local tactical bubble).",
            );
            return Ok(());
        };
        let observations = observe_bubble_contacts(active, hull, at_ic);
        let detected: Vec<&SpaceContactObservation> = observations
            .iter()
            .filter(|obs| obs.outcome.is_detected())
            .collect();
        if detected.is_empty() {
            vec!["Sensor pass: 0 contacts detected.".to_owned()]
        } else {
            let mut out = vec![format!(
                "Sensor pass: {} contact(s) detected:",
                detected.len()
            )];
            for obs in detected {
                let margin = match obs.outcome {
                    DetectionOutcome::Detected { margin, .. } => margin,
                    DetectionOutcome::Missed { .. } => 0,
                };
                out.push(format!(
                    "  [{}] ({}) range {} m, light-lag {} ms, IR {}, margin +{} (EMCON {})",
                    obs.ship_id,
                    obs.callsign,
                    obs.dist_m,
                    obs.light_lag_ms,
                    obs.ir_signature,
                    margin,
                    obs.emcon.code_str()
                ));
            }
            out
        }
    };
    for line in lines {
        ctx.out.sys_to_actor(ctx.actor, "tacsim.space", line);
    }
    push_space_panels(ctx, hull);
    Ok(())
}

fn resolve_detected_contact(
    w: &World,
    hull: ObjId,
    explicit_target: Option<&str>,
) -> Result<(String, String), CmdError> {
    let at_ic = w.world_ic_now().at;
    let book = w
        .module_state::<SpaceBubbleBook>()
        .ok_or_else(|| CmdError::msg("No active space bubble."))?;
    let active = book
        .active_for_hull(hull)
        .ok_or_else(|| CmdError::msg("No active space bubble for this ship."))?;
    let own_ship = active
        .bubble
        .ships
        .get(&active.carrier_ship_id)
        .ok_or_else(|| CmdError::msg("Carrier ship state missing from bubble."))?;
    if matches!(own_ship.status, HullStatus::DisabledDerelict { .. }) {
        return Err(CmdError::msg("This ship is disabled."));
    }

    let wanted = explicit_target
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(own_ship.locked_target_id.as_deref())
        .ok_or_else(|| CmdError::msg("Specify a target contact or acquire a +tac/lock first."))?;

    let observations = observe_bubble_contacts(active, hull, at_ic);
    let matched = observations
        .iter()
        .find(|obs| {
            obs.outcome.is_detected()
                && (obs.ship_id.eq_ignore_ascii_case(wanted)
                    || obs.callsign.eq_ignore_ascii_case(wanted))
        })
        .ok_or_else(|| CmdError::msg(NO_SUCH_CONTACT))?;

    Ok((active.carrier_ship_id.clone(), matched.ship_id.clone()))
}

fn verify_hostile_fire_allowed(
    w: &World,
    hull: ObjId,
    attacker_id: &str,
    target_id: &str,
) -> Result<(), CmdError> {
    if attacker_id == target_id {
        return Err(CmdError::msg("Cannot fire on your own ship."));
    }
    let Some(book) = w.module_state::<SpaceBubbleBook>() else {
        return Err(CmdError::msg("No active space bubble."));
    };
    let Some(active) = book.active_for_hull(hull) else {
        return Err(CmdError::msg("No active space bubble."));
    };
    let (Some(attacker), Some(target)) = (
        active.bubble.ships.get(attacker_id),
        active.bubble.ships.get(target_id),
    ) else {
        return Err(CmdError::msg(NO_SUCH_CONTACT));
    };
    if active.bubble.party_relation(attacker.party, target.party) == SpaceRelation::Friendly {
        return Err(CmdError::msg("Cannot fire on a friendly contact."));
    }
    Ok(())
}

fn tac_lock(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("sensors"))?;
    let raw = ctx.args.raw().trim();
    if raw.is_empty() {
        return Err(CmdError::msg("Usage: +tac/lock <contact_id|off>"));
    }
    if raw.eq_ignore_ascii_case("off") || raw.eq_ignore_ascii_case("none") {
        let Some((ship_id, _)) = active_carrier_ship_and_roll(ctx.w, hull) else {
            return Err(CmdError::msg("No active space bubble for this ship."));
        };
        let _ = apply_bubble_command(
            ctx.w,
            hull,
            BubbleInputCommand::SetLockedTarget {
                ship_id,
                target_id: None,
            },
        );
        ctx.out
            .sys_to_actor(ctx.actor, "tacsim.space", "Target lock cleared.");
        push_space_panels(ctx, hull);
        return Ok(());
    }

    let (ship_id, target_id) = resolve_detected_contact(ctx.w, hull, Some(raw))?;
    let _ = apply_bubble_command(
        ctx.w,
        hull,
        BubbleInputCommand::SetLockedTarget {
            ship_id,
            target_id: Some(target_id.clone()),
        },
    );
    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!("Target lock acquired on `{target_id}`."),
    );
    push_space_panels(ctx, hull);
    Ok(())
}

fn tac_beam(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("fire"))?;
    let raw = ctx.args.raw().trim();
    let (shooter_id, target_id) =
        resolve_detected_contact(ctx.w, hull, (!raw.is_empty()).then_some(raw))?;
    verify_hostile_fire_allowed(ctx.w, hull, &shooter_id, &target_id)?;

    let events = apply_bubble_command(
        ctx.w,
        hull,
        BubbleInputCommand::FireOffensiveLasers {
            shooter_id,
            target_id: target_id.clone(),
        },
    );
    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "Laser battery fired on `{target_id}` ({} combat event(s)).",
            events.len()
        ),
    );
    push_space_panels(ctx, hull);
    Ok(())
}

fn tac_torp(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("fire"))?;
    let raw = ctx.args.raw().trim();
    let (target_arg, count) = match raw.split_once('=') {
        Some((lhs, rhs)) => {
            let n: u8 = rhs
                .trim()
                .parse()
                .map_err(|_| CmdError::msg("Salvo count must be 1..=8."))?;
            if !(1..=8).contains(&n) {
                return Err(CmdError::msg("Salvo count must be 1..=8."));
            }
            ((!lhs.trim().is_empty()).then_some(lhs.trim()), n)
        }
        None => ((!raw.is_empty()).then_some(raw), 1),
    };
    let (shooter_id, target_id) = resolve_detected_contact(ctx.w, hull, target_arg)?;
    verify_hostile_fire_allowed(ctx.w, hull, &shooter_id, &target_id)?;

    let _ = apply_bubble_command(
        ctx.w,
        hull,
        BubbleInputCommand::FireTorpedoSalvo {
            shooter_id,
            target_id: target_id.clone(),
            count,
        },
    );
    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!("Launched torpedo salvo (x{count}) at `{target_id}`."),
    );
    push_space_panels(ctx, hull);
    Ok(())
}

fn tac_railgun(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("fire"))?;
    let raw = ctx.args.raw().trim();
    let (shooter_id, target_id) =
        resolve_detected_contact(ctx.w, hull, (!raw.is_empty()).then_some(raw))?;
    verify_hostile_fire_allowed(ctx.w, hull, &shooter_id, &target_id)?;

    let _ = apply_bubble_command(
        ctx.w,
        hull,
        BubbleInputCommand::FireRailgun {
            shooter_id,
            target_id: target_id.clone(),
        },
    );
    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!("Railgun salvo fired at `{target_id}`."),
    );
    push_space_panels(ctx, hull);
    Ok(())
}

fn tac_pdc(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("fire"))?;
    let raw = ctx.args.raw().trim().to_ascii_lowercase();
    let Some((ship_id, _)) = active_carrier_ship_and_roll(ctx.w, hull) else {
        return Err(CmdError::msg("No active space bubble for this ship."));
    };
    match raw.as_str() {
        "auto" | "on" => {
            let _ = apply_bubble_command(
                ctx.w,
                hull,
                BubbleInputCommand::SetPdcAuto {
                    ship_id,
                    enabled: true,
                },
            );
            ctx.out
                .sys_to_actor(ctx.actor, "tacsim.space", "PDC auto-intercept enabled.");
        }
        "manual" | "off" => {
            let _ = apply_bubble_command(
                ctx.w,
                hull,
                BubbleInputCommand::SetPdcAuto {
                    ship_id,
                    enabled: false,
                },
            );
            ctx.out
                .sys_to_actor(ctx.actor, "tacsim.space", "PDC auto-intercept disabled.");
        }
        "point" | "defense" | "defence" => {
            set_all_laser_modes(ctx.w, hull, &ship_id, LaserMode::Defense);
            ctx.out.sys_to_actor(
                ctx.actor,
                "tacsim.space",
                "Dual-mode laser mounts switched to Defense.",
            );
        }
        "offense" | "offence" => {
            set_all_laser_modes(ctx.w, hull, &ship_id, LaserMode::Offense);
            ctx.out.sys_to_actor(
                ctx.actor,
                "tacsim.space",
                "Dual-mode laser mounts switched to Offense.",
            );
        }
        _ => {
            return Err(CmdError::msg("Usage: +tac/pdc <auto|manual|point|offense>"));
        }
    }
    push_space_panels(ctx, hull);
    Ok(())
}

fn set_all_laser_modes(w: &mut World, hull: ObjId, ship_id: &str, mode: LaserMode) {
    let laser_ids: Vec<String> = w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| b.active_for_hull(hull))
        .and_then(|a| a.bubble.ships.get(ship_id))
        .map(|s| s.lasers.iter().map(|m| m.id.clone()).collect())
        .unwrap_or_default();
    for laser_id in laser_ids {
        let _ = apply_bubble_command(
            w,
            hull,
            BubbleInputCommand::SetLaserMode {
                ship_id: ship_id.to_owned(),
                laser_id,
                mode,
            },
        );
    }
}

#[allow(dead_code)]
fn _keep_bubble_import(_: &LocalSpaceBubble) {}
