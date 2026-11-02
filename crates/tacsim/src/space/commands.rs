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
use mush_sim::tick::{
    MAX_ORDER_SCALAR, MIN_ORDER_SCALAR, OrderChannel, OrderValue, StandingOrder,
};
use mush_sim::{DomainId, JourneyBook, SpatialState, StationId};

use super::bubble::{
    BubbleInputCommand, EmconPosture, HullStatus, LocalSpaceBubble, SpaceRelation, SpaceShipState,
    SubsystemCrit,
};
use super::tick::{ActiveInterdictionBubble, SpaceBubbleBook, apply_bubble_command};
use super::weapons::LaserMode;

/// The helm command verb (`+helm`).
pub const HELM_VERB: &str = "+helm";

/// All switches supported by `+helm`.
pub const HELM_SWITCHES: [&str; 7] = [
    "status", "station", "heading", "thrust", "emcon", "roll", "guidance",
];

/// Help text for `+helm`.
pub const HELM_HELP: &str = "\
+helm  --  Ship helm, bridge stations, and standing orders (doc 35 §5, D-35-9).
+helm[/status]                        Show bridge stations, standing orders, and flight state.
+helm/station <seat>[=<crew>]         Assign an aboard crew member (or yourself) to a bridge station
                                      (command|helm|tactical|ew|engineering|flight_ops), or `off`.
+helm/heading <yaw_deg>[,<pitch_deg>] Set nose attitude and write StandingOrder::Heading.
+helm/thrust <permille>[,<x>,<y>,<z>] Set main-drive throttle (0..1000) and write StandingOrder::Thrust.
+helm/emcon <ghost|passive|standard|combat>
                                      Set emission posture and write StandingOrder::Posture.
+helm/roll <roll_deg>                 Set roll angle in degrees (-180..180).
+helm/guidance <attitude_deg>         Set guidance yaw attitude in degrees (0..359).";

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
/// Checks both port-registered hulls ([`crate::port::ship::hull_of_room`] +
/// [`crate::port::voyage::aboard`]) and any carrier hull in [`SpatialState`] /
/// [`SpaceBubbleBook`] via [`crate::detachment::planning::projection::physically_aboard`].
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
    for grant in w.command_subjects_of(actor) {
        if grant.subject == hull
            && let Some(role) = BridgeRole::parse(grant.station.as_str())
                .or_else(|| BridgeRole::parse(grant.relation.as_str()))
        {
            return Some(role);
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
    let role =
        seat_role_for_actor(ctx.w, hull, ctx.actor).ok_or_else(|| CmdError::msg(NO_BRIDGE_STATION))?;
    if let Some(channel) = required_channel {
        let has_via_grant = ctx
            .w
            .command_subjects_of(ctx.actor)
            .iter()
            .any(|g| {
                g.subject == hull
                    && ChannelId::new(channel).is_some_and(|cid| g.channels.contains(&cid))
            });
        if !has_via_grant && !role.grants_channel(channel) {
            return Err(CmdError::msg(format!(
                "Your bridge station ({}) does not control the {channel} channel.",
                role.station_token()
            )));
        }
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
            seat_parts.push(format!("{}: {}", sid.as_str(), display_name(ctx.w, seat.actor)));
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
    }

    for line in lines {
        ctx.out.sys_to_actor(ctx.actor, "tacsim.space", line);
    }
    push_space_panels(ctx, hull);
    Ok(())
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
        Some(who) => mush_core::match_object(ctx.w, ctx.actor, who)?,
    };
    if hull_aboard_for_actor(ctx.w, assignee) != Some(hull) {
        return Err(CmdError::msg(
            "That person is not physically aboard this ship.",
        ));
    }

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
        return Err(CmdError::msg("Usage: +helm/heading <yaw_deg>[,<pitch_deg>]"));
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
            "Usage: +helm/thrust <permille 0..1000>[,<dir_x>,<dir_y>,<dir_z>]",
        ));
    }
    let parts: Vec<&str> = raw
        .split([',', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
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
                force_into_reserve: false,
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
    let (level, posture) = parse_emcon_token(raw).ok_or_else(|| {
        CmdError::msg("Usage: +helm/emcon <ghost|passive|standard|combat>")
    })?;

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
    let yaw_deg: i32 = ctx
        .args
        .raw()
        .trim()
        .parse()
        .map_err(|_| CmdError::msg("Usage: +helm/guidance <attitude_deg 0..359>"))?;
    if !(0..360).contains(&yaw_deg) {
        return Err(CmdError::msg("Guidance attitude must be in 0..359 degrees."));
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
            return Err(CmdError::msg(
                "Usage: +tac/pdc <auto|manual|point|offense>",
            ));
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
