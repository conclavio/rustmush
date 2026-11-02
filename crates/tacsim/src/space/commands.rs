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
pub const HELM_SWITCHES: [&str; 17] = [
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
    "cluster",
    "approach",
    "match",
    "submit",
    "run",
    "dock",
    "tow",
];

/// Help text for `+helm`.
pub const HELM_HELP: &str = "\
+helm  --  Ship helm, bridge stations, and standing orders (doc 35 §5, §7, §8, D-35-6, D-35-9, D-35-11..14).
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
+helm/autopilot <port>                Plan and recouple onto a corridor passage from live (pos, vel).
+helm/cluster <cluster_id>            Enter an orbital mining cluster (`ceres_halo`, `kirkwood_gap_3_1`, `pallas_inclined_belt`, `saturn_b_ring_edge`).
+helm/approach <rock>                 Execute a braking approach toward rock `A`, `B`, or `C` inside the cluster.
+helm/match <rock>                    Lock RCS velocity match (`|Δv| < 5 m/s` within `2,000 m`) with rock `A`, `B`, or `C`.
+helm/submit                          Heave-to and submit cargo manifest to a customs picket inspection (`D-35-11`).
+helm/run                             Refuse customs inspection and run the picket (`D-35-11`).
+helm/dock [<port|derelict> | tape <session_id> <rle> | abort]
                                      Initiate, step, or verify a 60 fps 6-DOF docking approach (`D-35-14`).
+helm/tow [<derelict_id>]             Attach a salvage tow line to a hard-docked derelict (`D-35-12`).";

/// The asteroid field mining command verb (`+mine`, `SF.R5`, `D-35-13`).
pub const MINE_VERB: &str = "+mine";

/// All switches supported by `+mine`.
pub const MINE_SWITCHES: [&str; 6] = ["status", "ping", "stake", "contest", "laser", "drones"];

/// Help text for `+mine`.
pub const MINE_HELP: &str = "\
+mine  --  Asteroid field prospecting, claim tenure, and ore extraction (doc 35 §7, Q-35-3, D-35-13).
+mine[/status]                        Show active orbital cluster, matched rock, extraction cycles, and stow.ore hold.
+mine/ping [<rock>]                   Ping asteroid pockets (`A`, `B`, `C`) for spectral class, reserves, and tenure.
+mine/stake <rock> [<family>]         Stake a 168-day Freeport Registry claim on an open pinged asteroid block.
+mine/contest <rock>                  Contest a player claim within the 30-day Registry contest window.
+mine/laser <rock> [<family>]         Start a timed 60s mining-laser extraction cycle into the hull's stow.ore hold.
+mine/drones <rock> [<1..4>] [<family>]
                                      Launch up to 4 collector-drone sorties (`ExcursionId`) into stow.ore.";

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

/// Feed identifier for the 60 fps 6-DOF bow docking camera (`SF.5`, `SF.R7`, `D-35-14`).
pub const SPACE_DOCKING_FEED: &str = "space.docking";

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

/// Emit viewer-filtered `space.holotank`, `space.cockpit`, and `space.docking` panel frames to
/// `actor` (`D-35-10`, `D-35-14`).
fn push_space_panels(ctx: &mut Ctx<'_>, hull: ObjId) {
    let (bubble_payloads, docking_payload) = {
        let book = ctx.w.module_state::<SpaceBubbleBook>();
        let bubble_pair = book.and_then(|b| b.active_for_hull(hull)).map(|active| {
            (
                active.bubble.holotank_feed(&active.carrier_ship_id),
                active.bubble.cockpit_feed(&active.carrier_ship_id),
            )
        });
        let dock_json = book
            .and_then(|b| b.docking_sessions.get(&hull))
            .map(build_docking_panel_payload);
        (bubble_pair, dock_json)
    };
    if let Some((holotank_payload, cockpit_payload)) = bubble_payloads {
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
    if let Some(dock_data) = docking_payload {
        ctx.out.push(
            Audience::Actor(ctx.actor),
            OutputEvent::Panel {
                feed: SPACE_DOCKING_FEED.into(),
                kind: PanelFrameKind::Keyframe,
                data: dock_data,
            },
        );
    }
}

fn build_docking_panel_payload(session: &super::docking::DockingSession) -> serde_json::Value {
    let pose = &session.live_stepper.pose;
    let gates = super::docking::evaluate_docking_gates(pose);
    let (target_label, target_kind) = match &session.target {
        super::docking::DockingTargetKind::PortBerth { port_name, .. } => {
            (port_name.as_str(), "station")
        }
        super::docking::DockingTargetKind::DisabledDerelict { derelict_name, .. } => {
            (derelict_name.as_str(), "ship")
        }
    };
    let status = if gates.all_green() {
        "ALIGNED-READY"
    } else {
        "APPROACHING"
    };
    serde_json::json!({
        "session_id": session.challenge.session_id,
        "dx_mm": pose.lateral_x_mm,
        "dy_mm": pose.lateral_y_mm,
        "dz_mm": pose.range_z_mm,
        "pitch_mdeg": pose.pitch_err_mdeg,
        "yaw_mdeg": pose.yaw_err_mdeg,
        "roll_mdeg": pose.roll_err_mdeg,
        "vx_mmps": pose.vel_x_mmps,
        "vy_mmps": pose.vel_y_mmps,
        "vz_mmps": pose.closure_mmps,
        "pitch_rate_mdeg_s": pose.pitch_rate_mdeg_s,
        "yaw_rate_mdeg_s": pose.yaw_rate_mdeg_s,
        "roll_rate_mdeg_s": pose.roll_rate_mdeg_s,
        "target_label": target_label,
        "target_kind": target_kind,
        "paused_from_world_tick": session.paused_from_world_tick,
        "dampeners": session.live_stepper.dampeners_on,
        "status": status,
        "gates_green": gates.green_count(),
        "target_tumble_mdeg_s": session.challenge.target_tumble_mdeg_s,
    })
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
        Some("cluster") => helm_cluster(ctx),
        Some("approach") => helm_approach(ctx),
        Some("match") => helm_match(ctx),
        Some("submit") => helm_submit(ctx),
        Some("run") => helm_run(ctx),
        Some("dock") => helm_dock(ctx),
        Some("tow") => helm_tow(ctx),
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
        if let Some(session) = book.docking_sessions.get(&hull) {
            let gates = super::docking::evaluate_docking_gates(&session.live_stepper.pose);
            lines.push(format!(
                "  Docking session `{}` — range={} mm, closure={} mm/s, gates={}/5 green, paused_from_world_tick={}.",
                session.challenge.session_id,
                session.live_stepper.pose.range_z_mm,
                session.live_stepper.pose.closure_mmps,
                gates.green_count(),
                session.paused_from_world_tick
            ));
        }
        if let Some(tow) = book.active_tows.get(&hull) {
            lines.push(format!(
                "  Active tow: derelict #{} ({} t <= 2x {} t tug cap), 48h Registry salvage window expires at {} real s.",
                tow.derelict_hull.0,
                tow.derelict_dry_mass_t,
                tow.tug_dry_mass_t,
                tow.contest_expires_real_s
            ));
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
    match active.bubble.party_relation(attacker.party, target.party) {
        SpaceRelation::Hostile => Ok(()),
        SpaceRelation::Friendly => Err(CmdError::msg("Cannot fire on a friendly contact.")),
        SpaceRelation::Neutral | SpaceRelation::Inspecting => {
            Err(CmdError::msg("Cannot fire on a non-hostile contact."))
        }
    }
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

// ---------------------------------------------------------------------------
// Asteroid field mining (`SF.R5`, `doc 35` §7, `Q-35-3`, `D-35-13`)
// ---------------------------------------------------------------------------

/// Debit live reaction mass (`kg` and whole-tonne [`crate::port::burn::PendingSpaceBurn`]s) for a
/// cluster approach or velocity-match manoeuvre (`SF.R4`, `SF.R5`).
pub(crate) fn debit_hull_propellant_for_dv(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    dv_mps: u64,
    force_reserve: bool,
) -> Result<u64, CmdError> {
    if dv_mps == 0 {
        return Ok(0);
    }
    let (basis, live_kg) = live_hull_basis_and_propellant_kg(ctx, hull)?;
    let non_prop_payload = basis
        .payload
        .saturating_sub(basis.tank.saturating_mul(1_000));
    let effective_payload = non_prop_payload.saturating_add(live_kg);
    let dv_u32 = u32::try_from(dv_mps).unwrap_or(u32::MAX);
    let burn_kg = crate::port::passage::burn_kg_for_dv(
        &basis.spec.drive,
        basis.spec.dry_kg,
        effective_payload,
        dv_u32,
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;
    if burn_kg > live_kg {
        return Err(CmdError::msg(format!(
            "Insufficient propellant: manoeuvre requires {burn_kg} kg ({dv_mps} m/s Δv), tank holds {live_kg} kg."
        )));
    }
    if !force_reserve
        && let Some(ff) = ctx
            .w
            .module_state::<SpaceBubbleBook>()
            .and_then(|b| b.free_flight_for_hull(hull).cloned())
        && let Some((reserve, nearest_port)) = compute_hull_return_reserve(
            ctx.w,
            &basis,
            live_kg,
            ff.pos_km,
            ff.vel_mps,
            ctx.w.world_ic_now().at,
        )
    {
        let remaining_after = live_kg.saturating_sub(burn_kg);
        if remaining_after < reserve.reserve_kg {
            return Err(CmdError::msg(format!(
                "RETURN_RESERVE warning: manoeuvre requires {burn_kg} kg, leaving {remaining_after} kg below the {} kg return reserve to {nearest_port}.",
                reserve.reserve_kg
            )));
        }
    }
    let whole_tonnes = {
        let book = ctx
            .w
            .module_state_mut::<SpaceBubbleBook>()
            .ok_or_else(|| CmdError::msg("Space bubble state is not registered."))?;
        book.propellant_ledger_mut(hull)
            .record_burn_kg(basis.tank, burn_kg)
    };
    let _ = submit_space_burn_if_needed(ctx, hull, basis.vault.0, whole_tonnes, 0)?;
    Ok(burn_kg)
}

fn helm_cluster(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("movement"))?;
    let raw = ctx.args.raw().trim().to_owned();
    let cluster = super::mining::OrbitalClusterId::parse(&raw).ok_or_else(|| {
        CmdError::msg(
            "Usage: +helm/cluster <ceres_halo|kirkwood_gap_3_1|pallas_inclined_belt|saturn_b_ring_edge>",
        )
    })?;

    let _ = super::mining::ensure_cluster_laid_up(ctx, cluster).map_err(CmdError::msg)?;
    let now_ic = ctx.w.world_ic_now().at;

    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        let existing_drones = book
            .cluster_sessions
            .get(&hull)
            .map(|s| s.drone_things.clone())
            .unwrap_or_default();
        book.cluster_sessions.insert(
            hull,
            super::mining::ActiveClusterMiningSession {
                hull,
                cluster,
                // Enter at 2,800 m standoff and 12.0 m/s relative drift (`35a` §4 SF.13: outside
                // the 2,000 m / 5.0 m/s velocity-match gates until `+helm/approach` brakes).
                ship_pos_m: [3_800, 0, 0],
                ship_vel_mmps: [12_000, 0, 0],
                approaching_rock: None,
                matched_rock: None,
                pings: std::collections::BTreeMap::new(),
                active_laser: None,
                active_drones: Vec::new(),
                drone_things: existing_drones,
                frontier_bloom_heat: 0,
                claim_jump_flagged: false,
                last_ic_s: now_ic,
            },
        );
    }

    let (domain, generation, tick) = domain_stamp_for_hull(ctx.w, hull);
    let _ = record_hull_standing_order(
        ctx.w,
        hull,
        OrderChannel::Heading,
        heading_order_vector(0, 0),
        ctx.actor,
        domain,
        generation,
        tick,
    )?;

    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "Entered orbital cluster {} (`{}`): standoff pos=(3800, 0, 0) m, drift vel=(12.0, 0.0, 0.0) m/s. Run `+mine/ping` to survey pockets and `+helm/approach <A|B|C>` to close range.",
            cluster.display_name(),
            cluster.as_str()
        ),
    );
    push_space_panels(ctx, hull);
    Ok(())
}

fn helm_approach(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("movement"))?;
    let raw = ctx.args.raw().trim();
    if raw.is_empty() {
        return Err(CmdError::msg("Usage: +helm/approach <A|B|C>"));
    }
    let cluster = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| b.cluster_sessions.get(&hull))
        .map(|s| s.cluster)
        .ok_or_else(|| {
            CmdError::msg("Ship is not in an orbital cluster; use `+helm/cluster <id>` first.")
        })?;
    let spec = cluster.rock(raw).ok_or_else(|| {
        CmdError::msg(format!(
            "Unknown asteroid pocket `{raw}` in {} (choose A, B, or C).",
            cluster.display_name()
        ))
    })?;

    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
        && let Some(session) = book.cluster_sessions.get_mut(&hull)
    {
        session.approaching_rock = Some(spec.letter);
        session.matched_rock = None;
    }

    let (domain, generation, tick) = domain_stamp_for_hull(ctx.w, hull);
    let order = record_hull_standing_order(
        ctx.w,
        hull,
        OrderChannel::Thrust,
        OrderValue::Scalar { value: 350 },
        ctx.actor,
        domain,
        generation,
        tick,
    )?;

    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "Braking approach ordered toward Block {} (`{}` {}) in {} (standing order `{}` rev {}).",
            spec.letter,
            spec.alias,
            spec.spectral.code(),
            cluster.display_name(),
            order.channel().token(),
            order.revision()
        ),
    );
    push_space_panels(ctx, hull);
    Ok(())
}

fn helm_match(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("movement"))?;
    let raw = ctx.args.raw().trim();
    if raw.is_empty() {
        return Err(CmdError::msg("Usage: +helm/match <A|B|C>"));
    }
    let (cluster, ship_pos_m, ship_vel_mmps) = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| b.cluster_sessions.get(&hull))
        .map(|s| (s.cluster, s.ship_pos_m, s.ship_vel_mmps))
        .ok_or_else(|| {
            CmdError::msg("Ship is not in an orbital cluster; use `+helm/cluster <id>` first.")
        })?;
    let spec = cluster.rock(raw).ok_or_else(|| {
        CmdError::msg(format!(
            "Unknown asteroid pocket `{raw}` in {} (choose A, B, or C).",
            cluster.display_name()
        ))
    })?;

    // Strict 3D Euclidean range (`<= 2,000 m`) and 3D relative speed (`< 5,000 mm/s = 5.0 m/s`)
    // check — never teleports from arbitrary distance (`35a` §5 #11)!
    let (dist_m, rel_speed_mmps) = super::mining::check_velocity_match_gate(
        ship_pos_m,
        ship_vel_mmps,
        spec.pos_m,
        spec.vel_mmps,
    )
    .map_err(CmdError::msg)?;

    let dv_mps = ((rel_speed_mmps as u64).div_ceil(1_000)).max(1);
    let burned_kg = debit_hull_propellant_for_dv(ctx, hull, dv_mps, true)?;

    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
        && let Some(session) = book.cluster_sessions.get_mut(&hull)
    {
        session.ship_vel_mmps = spec.vel_mmps;
        session.approaching_rock = None;
        session.matched_rock = Some(spec.letter);
    }

    let (domain, generation, tick) = domain_stamp_for_hull(ctx.w, hull);
    let order = record_hull_standing_order(
        ctx.w,
        hull,
        OrderChannel::Thrust,
        OrderValue::Scalar { value: 0 },
        ctx.actor,
        domain,
        generation,
        tick,
    )?;

    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "RCS velocity match locked on Block {} (`{}` {}) at range {dist_m} m (nulled {:.2} m/s drift, burned {burned_kg} kg reaction mass; standing order `{}` rev {}).",
            spec.letter,
            spec.alias,
            spec.spectral.code(),
            (rel_speed_mmps as f64) / 1_000.0,
            order.channel().token(),
            order.revision()
        ),
    );
    push_space_panels(ctx, hull);
    Ok(())
}

fn helm_submit(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("movement"))?;
    let now_ic = ctx.w.world_ic_now().at;

    let (corvette_ship_id, carrier_ship_id, rel_speed_mmps, status) = {
        let book = ctx
            .w
            .module_state::<SpaceBubbleBook>()
            .ok_or_else(|| CmdError::msg("No active space bubble."))?;
        let active = book
            .active_for_hull(hull)
            .ok_or_else(|| CmdError::msg("No active space bubble for this ship."))?;
        let picket = active
            .picket
            .as_ref()
            .ok_or_else(|| CmdError::msg("No customs picket is hailing this ship."))?;
        let carrier = active
            .bubble
            .ships
            .get(&active.carrier_ship_id)
            .ok_or_else(|| CmdError::msg("Carrier ship state missing from bubble."))?;
        let corvette = active
            .bubble
            .ships
            .get(&picket.corvette_ship_id)
            .ok_or_else(|| CmdError::msg("Customs Corvette is no longer in the bubble."))?;
        let rel_speed = super::picket::relative_speed_3d_mmps(carrier.vel_mmps, corvette.vel_mmps);
        (
            picket.corvette_ship_id.clone(),
            active.carrier_ship_id.clone(),
            rel_speed,
            picket.inspection_status,
        )
    };

    if matches!(
        status,
        super::picket::PicketInspectionStatus::Cleared
            | super::picket::PicketInspectionStatus::ClearedAfterFine
    ) {
        return Err(CmdError::msg(
            "Republic Customs picket has already cleared this ship.",
        ));
    }
    if status == super::picket::PicketInspectionStatus::RanThePicket {
        return Err(CmdError::msg(
            "You already ran the Republic Customs picket; inspection is no longer available.",
        ));
    }
    if rel_speed_mmps >= super::picket::HEAVE_TO_REL_VEL_MMPS {
        return Err(CmdError::msg(format!(
            "Must heave-to under {:.1} m/s relative speed (`+helm/thrust`) before submitting to customs inspection (current relative speed {:.1} m/s).",
            (super::picket::HEAVE_TO_REL_VEL_MMPS as f64) / 1_000.0,
            (rel_speed_mmps as f64) / 1_000.0
        )));
    }

    let manifest = super::picket::inspect_hull_manifest(ctx, hull, Some(ctx.actor), now_ic);

    if manifest.unlicensed_strategic_ore_kg > 0 {
        let (contract_id, fine_minor, front_id) =
            super::picket::assess_customs_fine_and_advance_front(
                ctx,
                ctx.actor,
                hull,
                manifest.unlicensed_strategic_ore_kg,
            )
            .map_err(CmdError::msg)?;
        let front_raw = front_id.map(|f| f.0);

        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
            && let Some(active) = book.by_hull.get_mut(&hull)
        {
            active.bubble.set_party_relation(
                super::picket::CUSTOMS_CORVETTE_PARTY,
                super::tick::CARRIER_PARTY,
                SpaceRelation::Friendly,
            );
            if let Some(picket) = active.picket.as_mut() {
                picket.inspection_status = super::picket::PicketInspectionStatus::ClearedAfterFine;
                picket.inspection_outcome = Some(super::picket::PicketInspectionOutcome::Fined {
                    contract_id: contract_id.0,
                    fine_minor,
                    unlicensed_strategic_ore_kg: manifest.unlicensed_strategic_ore_kg,
                    front_id: front_raw,
                });
            }
        }

        let fine_str = crate::port::coop::money_minor(ctx, fine_minor);
        let line = format!(
            "[PICKET] Republic Customs Corvette `{corvette_ship_id}` inspected `{carrier_ship_id}` manifest: {} kg unlicensed `strategic_ore` detected; assessed {fine_str} contraband fine (Ledger contract #{}, campaign front `{}` #{}) and granted customs clearance.",
            manifest.unlicensed_strategic_ore_kg,
            contract_id.0,
            mush_faction::CUSTOMS_ASSERTION_TITLE,
            front_raw.unwrap_or(0)
        );
        crate::port::voyage::narrate_hull(ctx, hull, &line);
        ctx.out.sys_to_actor(ctx.actor, "tacsim.space", line);
    } else {
        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
            && let Some(active) = book.by_hull.get_mut(&hull)
        {
            active.bubble.set_party_relation(
                super::picket::CUSTOMS_CORVETTE_PARTY,
                super::tick::CARRIER_PARTY,
                SpaceRelation::Friendly,
            );
            if let Some(picket) = active.picket.as_mut() {
                picket.inspection_status = super::picket::PicketInspectionStatus::Cleared;
                picket.inspection_outcome = Some(super::picket::PicketInspectionOutcome::Cleared {
                    strategic_ore_kg: manifest.strategic_ore_kg,
                });
            }
        }

        let line = format!(
            "[PICKET] Republic Customs Corvette `{corvette_ship_id}` inspected `{carrier_ship_id}` manifest ({} kg `strategic_ore`, licensed/compliant): customs clearance granted.",
            manifest.strategic_ore_kg
        );
        crate::port::voyage::narrate_hull(ctx, hull, &line);
        ctx.out.sys_to_actor(ctx.actor, "tacsim.space", line);
    }

    push_space_panels(ctx, hull);
    Ok(())
}

fn helm_run(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("movement"))?;
    let has_picket = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| b.active_for_hull(hull))
        .is_some_and(|a| a.picket.is_some());
    if !has_picket {
        return Err(CmdError::msg("No customs picket is hailing this ship."));
    }

    let front_id = super::picket::record_ran_the_picket_incident(ctx, ctx.actor, hull).map(|f| f.0);

    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
        && let Some(active) = book.by_hull.get_mut(&hull)
    {
        active.bubble.set_party_relation(
            super::picket::CUSTOMS_CORVETTE_PARTY,
            super::tick::CARRIER_PARTY,
            SpaceRelation::Hostile,
        );
        if let Some(picket) = active.picket.as_mut() {
            picket.inspection_status = super::picket::PicketInspectionStatus::RanThePicket;
            picket.inspection_outcome =
                Some(super::picket::PicketInspectionOutcome::RanThePicket { front_id });
        }
    }

    let line = format!(
        "[PICKET] You refuse inspection and run the Republic Customs picket! Corvette transitions to Hostile; campaign front `{}` #{} advanced.",
        mush_faction::CUSTOMS_ASSERTION_TITLE,
        front_id.unwrap_or(0)
    );
    crate::port::voyage::narrate_hull(ctx, hull, &line);
    ctx.out.sys_to_actor(ctx.actor, "tacsim.space", line);
    push_space_panels(ctx, hull);
    Ok(())
}

// ---------------------------------------------------------------------------
// 60 fps 6-DOF Docking Mini-Game, Input-Tape Verification & Salvage Tow (`SF.R7`)
// ---------------------------------------------------------------------------

fn debit_hull_propellant_kg(ctx: &mut Ctx<'_>, hull: ObjId, spent_kg: u32) -> Result<(), CmdError> {
    if spent_kg == 0 {
        return Ok(());
    }
    let Ok((basis, live_kg)) = live_hull_basis_and_propellant_kg(ctx, hull) else {
        return Ok(());
    };
    let clamped_kg = u64::from(spent_kg).min(live_kg);
    if clamped_kg == 0 {
        return Ok(());
    }
    let whole_tonnes = {
        let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() else {
            return Ok(());
        };
        let ledger = book.propellant_ledger_mut(hull);
        ledger.record_burn_kg(basis.tank, clamped_kg)
    };
    let _ = submit_space_burn_if_needed(ctx, hull, basis.vault.0, whole_tonnes, 0)?;
    Ok(())
}

fn apply_collar_scrape_damage(ctx: &mut Ctx<'_>, hull: ObjId, front_armor_damage: u32, at_ic: i64) {
    if front_armor_damage == 0 {
        return;
    }
    let bubble_ship = {
        let mut out = None;
        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
            && let Some(active) = book.by_hull.get_mut(&hull)
            && let Some(ship) = active.bubble.ships.get_mut(&active.carrier_ship_id)
        {
            if let Some(hull_sec) = ship.sections.get_mut(&super::bubble::ShipSectionId::Hull)
                && let Some(front) = hull_sec.facets.get_mut(&super::bubble::FacetBand::Front)
            {
                *front = front.saturating_sub(front_armor_damage);
            }
            out = Some(ship.clone());
        }
        out
    };
    if let Some(ship) = bubble_ship {
        let _ = super::custody::check_in_space_unit(ctx, hull, &ship, at_ic);
        return;
    }

    // Not in an active bubble: check out or instantiate the hull's `TacticalUnit`, apply Front
    // facet scrape damage, and check it back in so `TacticalState` records the collar scrape (`D-35-14`).
    let ship_name = display_name(ctx.w, hull);
    if let Ok(mut s) = super::custody::check_out_space_unit(
        ctx,
        hull,
        "tacsim.unit.dray_p",
        1,
        format!("ship.{}", hull.0),
        ship_name,
        super::tick::CARRIER_PARTY,
        [0, 0, 0],
        [0, 0, 0],
    ) {
        if let Some(hull_sec) = s.sections.get_mut(&super::bubble::ShipSectionId::Hull)
            && let Some(front) = hull_sec.facets.get_mut(&super::bubble::FacetBand::Front)
        {
            *front = front.saturating_sub(front_armor_damage);
        }
        let _ = super::custody::check_in_space_unit(ctx, hull, &s, at_ic);
    }
}

fn unpause_hull_after_docking(ctx: &mut Ctx<'_>, hull: ObjId, now: &ClockStamp) {
    let has_active_bubble = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .is_some_and(|b| b.by_hull.contains_key(&hull));
    if !has_active_bubble {
        let _ = mush_sim::resume_carrier_journey(ctx, hull, now);
    }
}

fn resolve_docking_target(
    ctx: &Ctx<'_>,
    hull: ObjId,
    token: &str,
) -> Result<super::docking::DockingTargetKind, CmdError> {
    let trimmed = token.trim();
    let lower = trimmed.to_ascii_lowercase();

    // 1. Check disabled contacts in the hull's active bubble or `SpaceBubbleBook::derelicts`.
    if let Some(book) = ctx.w.module_state::<SpaceBubbleBook>() {
        if let Some(active) = book.active_for_hull(hull) {
            let parsed_obj = trimmed
                .strip_prefix('#')
                .or_else(|| trimmed.strip_prefix("ship."))
                .and_then(|s| s.parse::<u32>().ok())
                .map(ObjId);
            for (ship_id, ship) in &active.bubble.ships {
                if ship_id == &active.carrier_ship_id {
                    continue;
                }
                let contact_hull = if ship_id == &active.interdictor_ship_id {
                    active.interdictor_hull
                } else if let Some(picket) = &active.picket
                    && ship_id == &picket.raider_ship_id
                {
                    picket.raider_hull
                } else {
                    ObjId(hull.0.saturating_add(1_000))
                };
                let matches_token = trimmed.eq_ignore_ascii_case("derelict")
                    || ship_id.eq_ignore_ascii_case(trimmed)
                    || ship.callsign.to_ascii_lowercase().contains(&lower)
                    || parsed_obj == Some(contact_hull);
                if matches_token {
                    if !matches!(ship.status, HullStatus::DisabledDerelict { .. }) {
                        return Err(CmdError::msg(format!(
                            "Target `{}` (#{}) is still operational; disable its drive before initiating collar docking.",
                            ship.callsign, contact_hull.0
                        )));
                    }
                    return Ok(super::docking::DockingTargetKind::DisabledDerelict {
                        derelict_hull: contact_hull,
                        derelict_name: ship.callsign.clone(),
                    });
                }
            }
        }

        // Also check `SpaceBubbleBook::derelicts`:
        let parsed_obj = trimmed
            .strip_prefix('#')
            .or_else(|| trimmed.strip_prefix("ship."))
            .and_then(|s| s.parse::<u32>().ok())
            .map(ObjId);
        for &derelict_hull in book.derelicts.keys() {
            let dname = display_name(ctx.w, derelict_hull);
            if trimmed.eq_ignore_ascii_case("derelict")
                || parsed_obj == Some(derelict_hull)
                || dname.to_ascii_lowercase().contains(&lower)
            {
                return Ok(super::docking::DockingTargetKind::DisabledDerelict {
                    derelict_hull,
                    derelict_name: dname,
                });
            }
        }
    }

    // 2. Check open ports / berths (`open_port_places`).
    let open_ports = open_port_places(ctx.w);
    if let Some((place_id, port_label, _, _)) =
        open_ports.into_iter().find(|(place_id, label, _, def)| {
            place_id.as_str().eq_ignore_ascii_case(&lower)
                || def.venue.as_str().eq_ignore_ascii_case(&lower)
                || label.to_ascii_lowercase().contains(&lower)
                || lower.contains(&def.venue.as_str().to_ascii_lowercase())
        })
    {
        return Ok(super::docking::DockingTargetKind::PortBerth {
            port_id: place_id.as_str().to_owned(),
            port_name: port_label,
        });
    }

    Err(CmdError::msg(format!(
        "Unknown docking target `{trimmed}` (specify an open port berth or a disabled derelict hull)."
    )))
}

fn resolve_port_destination_parent(
    w: &World,
    hull: ObjId,
    port_id: &str,
) -> Result<(mush_sim::SpatialParent, mush_sim::DomainId), CmdError> {
    let lower = port_id.to_ascii_lowercase();
    let open_ports = open_port_places(w);
    if let Some((_, _, dest_host, _)) = open_ports.into_iter().find(|(place_id, label, _, def)| {
        place_id.as_str().eq_ignore_ascii_case(&lower)
            || def.venue.as_str().eq_ignore_ascii_case(&lower)
            || label.to_ascii_lowercase().contains(&lower)
    }) && let Some(dest_domain) = w
        .module_state::<SpatialState>()
        .and_then(|s| s.domain_of_host(dest_host))
        .map(|d| d.id)
    {
        let pose_val = mush_sim::passive_rules_value(serde_json::json!({}))
            .map_err(|e| CmdError::msg(e.to_string()))?;
        return Ok((
            mush_sim::SpatialParent::Direct {
                domain: dest_domain,
                pose: mush_sim::Pose::passive(pose_val),
            },
            dest_domain,
        ));
    }

    if let Some(journey) = w.module_state::<JourneyBook>().and_then(|jb| jb.get(hull)) {
        let dest = journey.destination().clone();
        let dom = match &dest {
            mush_sim::SpatialParent::Direct { domain, .. } => *domain,
            _ => DomainId(1),
        };
        return Ok((dest, dom));
    }

    Err(CmdError::msg(format!(
        "Port `{port_id}` has no active spatial domain for docking."
    )))
}

fn helm_dock(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("movement"))?;
    let raw = ctx.args.raw().trim().to_owned();
    let now = ctx.w.world_ic_now();

    if raw.is_empty() || raw.eq_ignore_ascii_case("status") {
        let Some(book) = ctx.w.module_state::<SpaceBubbleBook>() else {
            return Err(CmdError::msg("Space state is not registered."));
        };
        if let Some(session) = book.docking_sessions.get(&hull) {
            let pose = &session.live_stepper.pose;
            let gates = super::docking::evaluate_docking_gates(pose);
            ctx.out.sys_to_actor(
                ctx.actor,
                "tacsim.space",
                format!(
                    "Docking session `{}` (paused_from_world_tick={}): dx={} mm, dy={} mm, dz={} mm, vz={} mm/s, pitch={} mdeg, yaw={} mdeg, roll={} mdeg ({}/5 gates green).",
                    session.challenge.session_id,
                    session.paused_from_world_tick,
                    pose.lateral_x_mm,
                    pose.lateral_y_mm,
                    pose.range_z_mm,
                    pose.closure_mmps,
                    pose.pitch_err_mdeg,
                    pose.yaw_err_mdeg,
                    pose.roll_err_mdeg,
                    gates.green_count()
                ),
            );
            push_space_panels(ctx, hull);
            return Ok(());
        }
        if let Some(&derelict_hull) = book.clamped_derelicts.get(&hull) {
            ctx.out.sys_to_actor(
                ctx.actor,
                "tacsim.space",
                format!(
                    "Collar hard-docked to derelict #{} ({}); run `+helm/tow` to attach salvage tow line.",
                    derelict_hull.0,
                    display_name(ctx.w, derelict_hull)
                ),
            );
            return Ok(());
        }
        return Err(CmdError::msg(
            "No active docking session. Usage: +helm/dock <port|derelict> | +helm/dock tape <session_id> <rle_tape> | +helm/dock abort",
        ));
    }

    if raw.eq_ignore_ascii_case("abort") {
        let removed = ctx
            .w
            .module_state_mut::<SpaceBubbleBook>()
            .and_then(|b| b.docking_sessions.remove(&hull));
        let Some(session) = removed else {
            return Err(CmdError::msg("No active docking session to abort."));
        };
        let spent_kg = session
            .live_stepper
            .rcs_spent_kg(&session.challenge.rcs_authority);
        debit_hull_propellant_kg(ctx, hull, spent_kg)?;
        unpause_hull_after_docking(ctx, hull, &now);
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.space",
            format!(
                "Docking session `{}` aborted; hull unpaused and returned to live world tick (spent {spent_kg} kg RCS propellant).",
                session.challenge.session_id
            ),
        );
        push_space_panels(ctx, hull);
        return Ok(());
    }

    // Interactive single-frame text control order (`DOCK TRANS UP`, `+helm/dock order DOCK TRANS UP`, etc.):
    let maybe_order_str = raw.strip_prefix("order ").unwrap_or(raw.as_str()).trim();
    if let Some(control_input) = super::docking::parse_dock_text_order(maybe_order_str) {
        if control_input.abort {
            let _ = ctx
                .w
                .module_state_mut::<SpaceBubbleBook>()
                .and_then(|b| b.docking_sessions.remove(&hull));
            unpause_hull_after_docking(ctx, hull, &now);
            ctx.out.sys_to_actor(
                ctx.actor,
                "tacsim.space",
                "Docking approach aborted via control order; hull unpaused.".to_owned(),
            );
            push_space_panels(ctx, hull);
            return Ok(());
        }
        let pose_summary = {
            let book = ctx
                .w
                .module_state_mut::<SpaceBubbleBook>()
                .ok_or_else(|| CmdError::msg("Space state is not registered."))?;
            let session = book
                .docking_sessions
                .get_mut(&hull)
                .ok_or_else(|| CmdError::msg("No active docking session for this ship."))?;
            let auth = session.challenge.rcs_authority;
            let tumble = session.challenge.target_tumble_mdeg_s;
            session.live_stepper.step_frame(control_input, &auth, tumble);
            let gates = super::docking::evaluate_docking_gates(&session.live_stepper.pose);
            format!(
                "6-DOF RCS pulse applied (`{maybe_order_str}`): dx={} mm, dy={} mm, dz={} mm, vz={} mm/s ({}/5 gates green).",
                session.live_stepper.pose.lateral_x_mm,
                session.live_stepper.pose.lateral_y_mm,
                session.live_stepper.pose.range_z_mm,
                session.live_stepper.pose.closure_mmps,
                gates.green_count()
            )
        };
        ctx.out.sys_to_actor(ctx.actor, "tacsim.space", pose_summary);
        push_space_panels(ctx, hull);
        return Ok(());
    }

    // Tape verification submission (`+helm/dock tape <session_id> <rle_tape>` or `+helm/dock tape <rle_tape>`):
    if let Some(tape_tail) = raw.strip_prefix("tape").map(str::trim) {
        if tape_tail.is_empty() {
            return Err(CmdError::msg(
                "Usage: +helm/dock tape <session_id> <rle_tape>",
            ));
        }
        let session = ctx
            .w
            .module_state::<SpaceBubbleBook>()
            .and_then(|b| b.docking_sessions.get(&hull).cloned())
            .ok_or_else(|| CmdError::msg("No active docking session for this ship."))?;

        let parts: Vec<&str> = tape_tail.split_whitespace().collect();
        let (submitted_session_id, rle_str) = if parts.len() >= 2 {
            (Some(parts[0]), parts[1..].join(""))
        } else {
            (None, parts[0].to_owned())
        };
        if let Some(sub_id) = submitted_session_id
            && sub_id != session.challenge.session_id
        {
            return Err(CmdError::msg(
                super::docking::DockingTapeError::SessionIdMismatch {
                    expected: session.challenge.session_id.clone(),
                    got: sub_id.to_owned(),
                }
                .to_string(),
            ));
        }

        let tape = super::docking::DockingInputTape::decode_rle(&rle_str)
            .map_err(|e| CmdError::msg(e.to_string()))?;
        let report = super::docking::replay_docking_tape(&session.challenge, &tape)
            .map_err(|e| CmdError::msg(e.to_string()))?;

        match report.outcome {
            super::docking::DockingOutcome::OutOfGate { gates } => {
                return Err(CmdError::msg(format!(
                    "Docking input tape rejected: collar gates not aligned at end of {} frames ({}/5 gates green: lateral={}, attitude={}, roll={}, closure={}, range={}; final pose dx={} mm, dy={} mm, dz={} mm, vz={} mm/s).",
                    report.frames_replayed,
                    gates.green_count(),
                    gates.lateral_ok,
                    gates.attitude_ok,
                    gates.roll_ok,
                    gates.closure_ok,
                    gates.range_ok,
                    report.final_pose.lateral_x_mm,
                    report.final_pose.lateral_y_mm,
                    report.final_pose.range_z_mm,
                    report.final_pose.closure_mmps,
                )));
            }
            super::docking::DockingOutcome::Aborted => {
                if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
                    book.docking_sessions.remove(&hull);
                }
                debit_hull_propellant_kg(ctx, hull, report.rcs_spent_kg)?;
                unpause_hull_after_docking(ctx, hull, &now);
                ctx.out.sys_to_actor(
                    ctx.actor,
                    "tacsim.space",
                    format!(
                        "Docking session `{}` aborted on frame {} (spent {} kg RCS propellant); hull unpaused.",
                        session.challenge.session_id, report.frames_replayed, report.rcs_spent_kg
                    ),
                );
                push_space_panels(ctx, hull);
                return Ok(());
            }
            super::docking::DockingOutcome::CollarScrapeBounce {
                impact_closure_mmps,
                front_armor_damage,
            } => {
                if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
                    book.docking_sessions.remove(&hull);
                }
                debit_hull_propellant_kg(ctx, hull, report.rcs_spent_kg)?;
                apply_collar_scrape_damage(ctx, hull, front_armor_damage, now.at);
                unpause_hull_after_docking(ctx, hull, &now);
                let line = format!(
                    "[DOCKING] Hard collar bounce at {} mm/s on frame {}! Bow collar scraped (`Front` facet -{} armor, spent {} kg RCS propellant); docking session closed and hull unpaused.",
                    impact_closure_mmps,
                    report.frames_replayed,
                    front_armor_damage,
                    report.rcs_spent_kg
                );
                crate::port::voyage::narrate_hull(ctx, hull, &line);
                ctx.out.sys_to_actor(ctx.actor, "tacsim.space", line);
                push_space_panels(ctx, hull);
                return Ok(());
            }
            super::docking::DockingOutcome::Clamped => {
                if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
                    book.docking_sessions.remove(&hull);
                }
                debit_hull_propellant_kg(ctx, hull, report.rcs_spent_kg)?;

                match session.target {
                    super::docking::DockingTargetKind::PortBerth {
                        ref port_id,
                        ref port_name,
                    } => {
                        let (dest_parent, _) = resolve_port_destination_parent(ctx.w, hull, port_id)?;
                        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
                            book.free_flights.remove(&hull);
                            book.by_hull.remove(&hull);
                        }
                        mush_sim::dock_carrier_at_destination(ctx, hull, dest_parent, &now)
                            .map_err(CmdError::msg)?;
                        let line = format!(
                            "[DOCKING] Hard dock and magnetic collar seal confirmed at {port_name} on frame {} (dz={} mm, vz={} mm/s, spent {} kg RCS propellant; berth transitioned to `Berth::Docked`).",
                            report.frames_replayed,
                            report.final_pose.range_z_mm,
                            report.final_pose.closure_mmps,
                            report.rcs_spent_kg
                        );
                        crate::port::voyage::narrate_hull(ctx, hull, &line);
                        ctx.out.sys_to_actor(ctx.actor, "tacsim.space", line);
                    }
                    super::docking::DockingTargetKind::DisabledDerelict {
                        derelict_hull,
                        ref derelict_name,
                    } => {
                        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
                            book.clamped_derelicts.insert(hull, derelict_hull);
                        }
                        unpause_hull_after_docking(ctx, hull, &now);
                        let line = format!(
                            "[DOCKING] Hard dock and magnetic collar seal confirmed on derelict {derelict_name} (#{}) on frame {} (dz={} mm, vz={} mm/s, spent {} kg RCS propellant). Run `+helm/tow` to attach salvage tow line.",
                            derelict_hull.0,
                            report.frames_replayed,
                            report.final_pose.range_z_mm,
                            report.final_pose.closure_mmps,
                            report.rcs_spent_kg
                        );
                        crate::port::voyage::narrate_hull(ctx, hull, &line);
                        ctx.out.sys_to_actor(ctx.actor, "tacsim.space", line);
                    }
                }
                push_space_panels(ctx, hull);
                return Ok(());
            }
        }
    }

    // Initiate a new `DockingSession` (`+helm/dock <port|derelict>` or `+helm/dock start <port|derelict>`):
    let target_token = raw.strip_prefix("start ").unwrap_or(raw.as_str()).trim();
    let target = resolve_docking_target(ctx, hull, target_token)?;

    let session_id = ctx
        .w
        .module_state_mut::<SpaceBubbleBook>()
        .map(|b| b.alloc_docking_session_id(hull))
        .ok_or_else(|| CmdError::msg("Space state is not registered."))?;

    let challenge = match &target {
        super::docking::DockingTargetKind::PortBerth { port_id, .. } => {
            if let Ok((_, port_domain)) = resolve_port_destination_parent(ctx.w, hull, port_id) {
                let _ = mush_sim::promote_carrier_journey(ctx, hull, port_domain, &now);
            }
            super::docking::DockingChallenge::standard_port(session_id.clone())
        }
        super::docking::DockingTargetKind::DisabledDerelict { .. } => {
            super::docking::DockingChallenge::derelict_collar(session_id.clone())
        }
    };

    let live_stepper = super::docking::DockingStepperState::new(challenge.start_pose.clone());
    let target_desc = match &target {
        super::docking::DockingTargetKind::PortBerth { port_name, .. } => port_name.clone(),
        super::docking::DockingTargetKind::DisabledDerelict {
            derelict_hull,
            derelict_name,
        } => format!("{derelict_name} (#{})", derelict_hull.0),
    };
    let session = super::docking::DockingSession {
        hull_id: hull,
        target,
        challenge: challenge.clone(),
        live_stepper,
        paused_from_world_tick: true,
        started_ic_s: now.at,
    };

    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        book.docking_sessions.insert(hull, session);
    }

    let line = format!(
        "[DOCKING] Initiated 60 fps 6-DOF docking session `{session_id}` toward {target_desc}: hull stepped out of live world tick (`paused_from_world_tick=true`, start dz={} mm, dx={} mm, dy={} mm). Submit `+helm/dock tape {session_id} <rle_tape>` to clamp.",
        challenge.start_pose.range_z_mm,
        challenge.start_pose.lateral_x_mm,
        challenge.start_pose.lateral_y_mm,
    );
    crate::port::voyage::narrate_hull(ctx, hull, &line);
    ctx.out.sys_to_actor(ctx.actor, "tacsim.space", line);
    push_space_panels(ctx, hull);
    Ok(())
}

fn canonical_hull_dry_mass_t(w: &World, hull: ObjId, default_t: u32) -> u32 {
    if let Some(view) = crate::port::ship::hull_view(w, hull)
        && let Some(spec) = super::custody::unit_spec_for_id(view.unit.as_str())
        && let Some(log) = &spec.logistics
    {
        return log.dry_mass_t;
    }
    if let Some(book) = w.module_state::<SpaceBubbleBook>()
        && let Some(active) = book.active_for_hull(hull)
    {
        if hull == active.interdictor_hull
            && let Some(iship) = active.bubble.ships.get(&active.interdictor_ship_id)
        {
            return iship.dry_mass_t;
        }
        if let Some(cship) = active.bubble.ships.get(&active.carrier_ship_id) {
            return cship.dry_mass_t;
        }
    }
    default_t
}

fn helm_tow(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, Some("movement"))?;
    let raw = ctx.args.raw().trim().to_owned();
    let now = ctx.w.world_ic_now();

    let clamped_derelict = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| b.clamped_derelicts.get(&hull).copied())
        .ok_or_else(|| {
            CmdError::msg(
                "Must complete a 6-DOF hard-dock clamp (`+helm/dock <derelict>` then `+helm/dock tape ...`) before attaching a tow line.",
            )
        })?;

    if !raw.is_empty()
        && let Some(parsed_id) = raw
            .strip_prefix('#')
            .or_else(|| raw.strip_prefix("ship."))
            .and_then(|s| s.parse::<u32>().ok())
            .map(ObjId)
        && parsed_id != clamped_derelict
    {
        return Err(CmdError::msg(format!(
            "Ship is hard-docked to derelict #{}, not #{}.",
            clamped_derelict.0, parsed_id.0
        )));
    }

    let tug_dry_mass_t = canonical_hull_dry_mass_t(ctx.w, hull, 240);
    let derelict_dry_mass_t = canonical_hull_dry_mass_t(ctx.w, clamped_derelict, 180);
    let max_tow_mass_t = tug_dry_mass_t.saturating_mul(2);
    if derelict_dry_mass_t > max_tow_mass_t {
        return Err(CmdError::msg(format!(
            "Derelict #{} dry mass ({derelict_dry_mass_t} t) exceeds 2x tug dry-mass cap ({max_tow_mass_t} t for {tug_dry_mass_t} t tug; `doc 27/27a`).",
            clamped_derelict.0
        )));
    }

    let filed_real_s = if ctx.w.now() > 0 {
        (ctx.w.now() / 1_000).max(1)
    } else {
        (now.at / 24).max(1)
    };
    let contest_expires_real_s =
        filed_real_s.saturating_add(i64::from(super::bubble::DERELICT_WINDOW_S));

    // Ensure the derelict is registered in `SalvageClaimBook` and `SpaceBubbleBook::derelicts` (`D-35-12`):
    let already_claimed = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| b.derelicts.get(&clamped_derelict))
        .is_some_and(|d| d.salvage_title_status.is_some());
    if !already_claimed {
        let (domain, op_id) = {
            let book = ctx
                .w
                .module_state_mut::<SpaceBubbleBook>()
                .ok_or_else(|| CmdError::msg("Space state is not registered."))?;
            let dom = book
                .active_for_hull(hull)
                .map(|a| a.domain)
                .or_else(|| book.derelicts.get(&clamped_derelict).map(|d| d.domain))
                .unwrap_or(DomainId(1));
            book.next_recovery_op_id = book.next_recovery_op_id.saturating_add(1);
            let op = crate::field_ops::RecoveryOpId::new(book.next_recovery_op_id)
                .map_err(|e| CmdError::msg(e.to_string()))?;
            (dom, op)
        };
        let prior_owner = ObjId(clamped_derelict.0.saturating_add(1_000));
        let base_obj = ObjId(domain.0.try_into().unwrap_or(1u32).max(1));
        let title_status = ctx
            .w
            .module_state_mut::<crate::field_ops::SalvageClaimBook>()
            .and_then(|sb| {
                super::custody::register_derelict_salvage_in_book(
                    sb,
                    op_id,
                    clamped_derelict,
                    hull,
                    hull,
                    prior_owner,
                    base_obj,
                    now.at,
                    filed_real_s,
                )
                .ok()
            });
        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
            book.derelicts
                .entry(clamped_derelict)
                .or_insert(super::tick::SpaceDerelictRecord {
                    hull: clamped_derelict,
                    domain,
                    disabled_at_ic_s: now.at,
                    epoch_open: true,
                    life_support_remaining_s: super::bubble::DERELICT_WINDOW_S,
                    salvage_contest_window_s: super::bubble::DERELICT_WINDOW_S,
                    assessment: None,
                    salvage_title_status: title_status,
                });
        }
    }

    let tow_record = super::docking::SalvageTowRecord {
        tug_hull: hull,
        derelict_hull: clamped_derelict,
        tug_dry_mass_t,
        derelict_dry_mass_t,
        contest_expires_real_s,
    };
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        book.active_tows.insert(hull, tow_record);
    }

    let line = format!(
        "[SALVAGE] Tow line locked to disabled derelict #{} ({} t <= 2x {} t tug cap); 48-hour Freeport Registry salvage claim window active until {} real s (`SalvageClaimBook`).",
        clamped_derelict.0, derelict_dry_mass_t, tug_dry_mass_t, contest_expires_real_s
    );
    crate::port::voyage::narrate_hull(ctx, hull, &line);
    ctx.out.sys_to_actor(ctx.actor, "tacsim.space", line);
    push_space_panels(ctx, hull);
    Ok(())
}

/// Top-level command handler for `+mine` (`SF.R5`, `doc 35` §7, `Q-35-3`, `D-35-13`).
///
/// # Errors
///
/// Returns [`CmdError`] if `[tacsim].enabled` is false, the caller is not aboard a ship,
/// the caller lacks a bridge station or Freeport Registry prospecting licence, or the hull
/// is not velocity-matched to the target asteroid pocket.
pub fn mine_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    if ctx.switches.iter().count() > 1 {
        return Err(CmdError::msg("Choose one +mine switch."));
    }
    let switch = ctx.switches.iter().next();
    match switch {
        None | Some("status") => mine_status(ctx),
        Some("ping") => mine_ping(ctx),
        Some("stake") => mine_stake(ctx),
        Some("contest") => mine_contest(ctx),
        Some("laser") => mine_laser(ctx),
        Some("drones") => mine_drones(ctx),
        Some(other) => Err(CmdError::msg(format!("Unknown +mine switch: /{other}."))),
    }
}

fn mine_status(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, None)?;
    let used_t = super::mining::hull_hold_used_tonnes(ctx, hull);
    let Some(session) = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| b.cluster_sessions.get(&hull).cloned())
    else {
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.space",
            format!(
                "Mining status aboard {}: not in an orbital cluster (`stow.ore` hold: {used_t}/{} t).",
                display_name(ctx.w, hull),
                super::mining::DRAY_P_HOLD_CAPACITY_T
            ),
        );
        return Ok(());
    };

    let matched = session
        .matched_rock
        .map_or_else(|| "none".to_owned(), |c| format!("Block {c}"));
    let laser_desc = session.active_laser.as_ref().map_or_else(
        || "idle".to_owned(),
        |l| {
            format!(
                "Block {} ({}, {} t due @ {})",
                l.rock_letter,
                l.family.id(),
                l.yield_t,
                l.due_ic
            )
        },
    );
    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "Mining status aboard {} in {} (`{}`): pos=({}, {}, {}) m, vel=({}, {}, {}) mm/s, matched={matched}, laser={laser_desc}, active drones={}/{}, frontier bloom heat={}, `stow.ore` hold={used_t}/{} t ({} kg capacity).",
            display_name(ctx.w, hull),
            session.cluster.display_name(),
            session.cluster.as_str(),
            session.ship_pos_m[0],
            session.ship_pos_m[1],
            session.ship_pos_m[2],
            session.ship_vel_mmps[0],
            session.ship_vel_mmps[1],
            session.ship_vel_mmps[2],
            session.active_drones.len(),
            crate::surface::excursion::MAX_EXCURSIONS_PER_OPERATOR,
            session.frontier_bloom_heat,
            super::mining::DRAY_P_HOLD_CAPACITY_T,
            super::mining::DRAY_P_HOLD_CAPACITY_KG,
        ),
    );
    Ok(())
}

fn mine_ping(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, role) = require_aboard_and_seat(ctx, None)?;
    if !role.grants_channel("sensors") && !role.grants_channel("flight_ops") {
        return Err(CmdError::msg(format!(
            "Your bridge station ({}) does not control the sensors or flight_ops channel.",
            role.station_token()
        )));
    }
    let raw = ctx.args.raw().trim().to_owned();
    let now_ic = ctx.w.world_ic_now().at;

    let (session, site) = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| {
            let s = b.cluster_sessions.get(&hull)?.clone();
            let site = b.cluster_sites.get(&s.cluster)?.clone();
            Some((s, site))
        })
        .ok_or_else(|| {
            CmdError::msg("Ship is not in an orbital cluster; use `+helm/cluster <id>` first.")
        })?;

    let specs: Vec<&'static super::mining::ClusterRockSpec> = if raw.is_empty() {
        session.cluster.rocks().iter().collect()
    } else {
        let one = session.cluster.rock(&raw).ok_or_else(|| {
            CmdError::msg(format!(
                "Unknown asteroid pocket `{raw}` in {} (choose A, B, or C).",
                session.cluster.display_name()
            ))
        })?;
        vec![one]
    };

    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
        && let Some(sess_mut) = book.cluster_sessions.get_mut(&hull)
    {
        for spec in &specs {
            sess_mut
                .pings
                .entry((ctx.actor, spec.letter))
                .or_insert(now_ic);
        }
    }

    let mut lines = vec![format!(
        "Prospecting ping in {} ({:?} zone):",
        session.cluster.display_name(),
        session.cluster.zone()
    )];
    for spec in specs {
        let dx = i128::from(session.ship_pos_m[0]) - i128::from(spec.pos_m[0]);
        let dy = i128::from(session.ship_pos_m[1]) - i128::from(spec.pos_m[1]);
        let dz = i128::from(session.ship_pos_m[2]) - i128::from(spec.pos_m[2]);
        let dist_m = libm::sqrt((dx * dx + dy * dy + dz * dz) as f64).round() as i64;
        let dvx = i128::from(session.ship_vel_mmps[0]) - i128::from(spec.vel_mmps[0]);
        let dvy = i128::from(session.ship_vel_mmps[1]) - i128::from(spec.vel_mmps[1]);
        let dvz = i128::from(session.ship_vel_mmps[2]) - i128::from(spec.vel_mmps[2]);
        let rel_mps = libm::sqrt((dvx * dvx + dvy * dvy + dvz * dvz) as f64) / 1_000.0;

        let mut family_parts = Vec::new();
        if let Some(&rock_vault) = site.vaults.get(&spec.letter) {
            for &fam in spec.spectral.families() {
                let pool_t = super::mining::rock_pool_tonnes(ctx, rock_vault, fam);
                let sv = super::mining::ore_specific_volume_l_per_t(fam);
                family_parts.push(format!(
                    "{}={} t ({} L/t)",
                    crate::surface::opening::ore_good(fam),
                    pool_t,
                    sv
                ));
            }
        }
        let tenure_desc =
            super::mining::cluster_rock_tenure_word(ctx, session.cluster, spec, ctx.actor, now_ic);
        lines.push(format!(
            "  Block {} (`{}` {}): range {dist_m} m, rel speed {rel_mps:.2} m/s, reserves [{}], tenure: {tenure_desc}.",
            spec.letter,
            spec.alias,
            spec.spectral.code(),
            family_parts.join(", ")
        ));
    }

    for line in lines {
        ctx.out.sys_to_actor(ctx.actor, "tacsim.space", line);
    }
    Ok(())
}

fn require_prospecting_licence(w: &mush_core::World, actor: ObjId) -> Result<(), CmdError> {
    let now_ic = w.world_ic_now().at;
    crate::port::licence::licence_gate(w, actor, now_ic).map_err(|standing| {
        CmdError::msg(
            crate::port::licence::standing_line(w, actor, &standing)
                .unwrap_or_else(|| crate::port::tv::MINE_NO_LICENCE.to_owned()),
        )
    })
}

fn mine_stake(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, None)?;
    // Enforce the Freeport Registry prospecting licence (2,500 cr / 168 IC days, `D-35-13`).
    require_prospecting_licence(ctx.w, ctx.actor)?;
    if ctx.authoritative_pending(ctx.actor) {
        return Err(CmdError::msg(
            "The Registry already has a tenure filing in flight for you.",
        ));
    }

    let raw = ctx.args.raw().trim();
    let parts: Vec<&str> = raw.split_whitespace().collect();
    let rock_token = parts
        .first()
        .copied()
        .ok_or_else(|| CmdError::msg("Usage: +mine/stake <A|B|C> [<family>]"))?;
    let declared = match parts.get(1) {
        Some(tok) => Some(super::mining::parse_ore_family(tok).ok_or_else(|| {
            CmdError::msg(
                "Unknown ore family (choose metals, volatiles, carbon_feedstock, or strategic_elements).",
            )
        })?),
        None => None,
    };

    let session = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| b.cluster_sessions.get(&hull).cloned())
        .ok_or_else(|| {
            CmdError::msg("Ship is not in an orbital cluster; use `+helm/cluster <id>` first.")
        })?;
    let spec = session.cluster.rock(rock_token).ok_or_else(|| {
        CmdError::msg(format!(
            "Unknown asteroid pocket `{rock_token}` in {} (choose A, B, or C).",
            session.cluster.display_name()
        ))
    })?;

    if spec.tenure == crate::port::def::Tenure::Commons {
        return Err(CmdError::msg(
            crate::port::claims::STAKE_COMMONS
                .replace("{block}", &spec.letter.to_string())
                .replace("{site}", session.cluster.display_name()),
        ));
    }

    let now_ic = ctx.w.world_ic_now().at;
    let cluster_cid = session
        .cluster
        .canonical_id()
        .ok_or_else(|| CmdError::msg("Invalid cluster canonical id."))?;

    if let Some(handles) = ctx.w.module_state::<crate::port::families::PortHandles>() {
        let key = crate::surface::claims::BlockKey {
            site: cluster_cid.clone(),
            letter: spec.letter,
        };
        if let Ok(Some(record)) = ctx.action_state(&handles.families.claims, &key)
            && let Some(claim) = record.claim.as_ref()
            && claim.live(now_ic)
        {
            return Err(CmdError::msg(format!(
                "Block {} at {} is already held by {}; use `+mine/contest {}` within the 30-day contest window if eligible.",
                spec.letter,
                session.cluster.display_name(),
                crate::port::claims::holder_name(ctx.w, &claim.holder),
                spec.letter
            )));
        }
    }

    let discovered_ic = session
        .pings
        .get(&(ctx.actor, spec.letter))
        .copied()
        .or_else(|| (session.matched_rock == Some(spec.letter)).then_some(now_ic))
        .ok_or_else(|| {
            CmdError::msg(format!(
                "Ping Block {} (`+mine/ping {}`) before staking a Freeport Registry claim.",
                spec.letter, spec.letter
            ))
        })?;

    let receipt = format!(
        "tacsim.space.claim.stake.{cluster_cid}.{}.{}.{}",
        spec.letter.to_ascii_lowercase(),
        ctx.actor.0,
        now_ic.max(1)
    );
    let idem_key = mush_core::IdempotencyKeyText::parse_canonical(receipt.clone())
        .map_err(|_| CmdError::msg("Failed to form claim stake idempotency key."))?;
    let action = mush_industry::action_ref(ctx.reg, mush_industry::IndustryVerb::ClaimTenure)
        .ok_or_else(|| CmdError::msg("The Registry claim action is not wired."))?;

    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        book.cluster_tenure_orders.insert(
            ctx.actor,
            super::mining::ClusterTenureOrder {
                actor: ctx.actor,
                cluster: session.cluster,
                letter: spec.letter,
                is_contest: false,
                declared,
                discovered_ic,
                made_ic: now_ic,
                receipt: receipt.clone(),
            },
        );
    }

    ctx.submit_authoritative(
        action,
        mush_core::PerceivedRef::SelfActor,
        Vec::new(),
        serde_json::json!({ "receipt": receipt }),
        idem_key,
    );

    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "Filed Freeport Registry claim stake on Block {} at {} (168-day term, 30-day contest window).",
            spec.letter,
            session.cluster.display_name()
        ),
    );
    Ok(())
}

fn mine_contest(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, _) = require_aboard_and_seat(ctx, None)?;
    require_prospecting_licence(ctx.w, ctx.actor)?;
    if ctx.authoritative_pending(ctx.actor) {
        return Err(CmdError::msg(
            "The Registry already has a tenure filing in flight for you.",
        ));
    }

    let raw = ctx.args.raw().trim();
    if raw.is_empty() {
        return Err(CmdError::msg("Usage: +mine/contest <A|B|C>"));
    }
    let session = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| b.cluster_sessions.get(&hull).cloned())
        .ok_or_else(|| {
            CmdError::msg("Ship is not in an orbital cluster; use `+helm/cluster <id>` first.")
        })?;
    let spec = session.cluster.rock(raw).ok_or_else(|| {
        CmdError::msg(format!(
            "Unknown asteroid pocket `{raw}` in {} (choose A, B, or C).",
            session.cluster.display_name()
        ))
    })?;

    let now_ic = ctx.w.world_ic_now().at;
    let cluster_cid = session
        .cluster
        .canonical_id()
        .ok_or_else(|| CmdError::msg("Invalid cluster canonical id."))?;
    let handles = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .ok_or_else(|| CmdError::msg("Port handles are not registered."))?;
    let key = crate::surface::claims::BlockKey {
        site: cluster_cid.clone(),
        letter: spec.letter,
    };
    let record = ctx
        .action_state(&handles.families.claims, &key)
        .ok()
        .flatten()
        .ok_or_else(|| {
            CmdError::msg(format!(
                "Block {} at {} has no claim to contest.",
                spec.letter,
                session.cluster.display_name()
            ))
        })?;
    let claim = record.claim.as_ref().ok_or_else(|| {
        CmdError::msg(format!(
            "Block {} at {} has no active claim to contest.",
            spec.letter,
            session.cluster.display_name()
        ))
    })?;
    if !claim.live(now_ic) {
        return Err(CmdError::msg(format!(
            "The claim on Block {} has already lapsed; use `+mine/stake {}`.",
            spec.letter, spec.letter
        )));
    }
    if !claim.player_held() {
        return Err(CmdError::msg(format!(
            "Block {} is held by {}; organization concessions are not subject to Registry priority contest.",
            spec.letter,
            crate::port::claims::holder_name(ctx.w, &claim.holder)
        )));
    }
    if claim.holder == mush_org::PrincipalId::Object(ctx.actor) {
        return Err(CmdError::msg("You already hold that claim."));
    }
    // Enforce the 30-IC-day (`CONTEST_WINDOW_IC_SECS = 30 * 86_400`) contest window (`35a` §4 SF.12).
    if !crate::surface::tenure::contest_open(claim.registered_ic, now_ic) {
        return Err(CmdError::msg(format!(
            "The 30-day Registry contest window on Block {} at {} has closed.",
            spec.letter,
            session.cluster.display_name()
        )));
    }

    let discovered_ic = session
        .pings
        .get(&(ctx.actor, spec.letter))
        .copied()
        .unwrap_or(now_ic);
    let receipt = format!(
        "tacsim.space.claim.contest.{cluster_cid}.{}.{}.{}",
        spec.letter.to_ascii_lowercase(),
        ctx.actor.0,
        now_ic.max(1)
    );
    let idem_key = mush_core::IdempotencyKeyText::parse_canonical(receipt.clone())
        .map_err(|_| CmdError::msg("Failed to form claim contest idempotency key."))?;
    let action = mush_industry::action_ref(ctx.reg, mush_industry::IndustryVerb::ClaimTenure)
        .ok_or_else(|| CmdError::msg("The Registry claim action is not wired."))?;

    let declared = claim.declared;
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        book.cluster_tenure_orders.insert(
            ctx.actor,
            super::mining::ClusterTenureOrder {
                actor: ctx.actor,
                cluster: session.cluster,
                letter: spec.letter,
                is_contest: true,
                declared,
                discovered_ic,
                made_ic: now_ic,
                receipt: receipt.clone(),
            },
        );
    }

    ctx.submit_authoritative(
        action,
        mush_core::PerceivedRef::SelfActor,
        Vec::new(),
        serde_json::json!({ "receipt": receipt }),
        idem_key,
    );

    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "Filed Freeport Registry contest on Block {} at {} within the 30-day contest window.",
            spec.letter,
            session.cluster.display_name()
        ),
    );
    Ok(())
}

fn mine_laser(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, role) = require_aboard_and_seat(ctx, None)?;
    if !role.grants_channel("flight_ops") && !role.grants_channel("fire") {
        return Err(CmdError::msg(format!(
            "Your bridge station ({}) does not control the flight_ops or fire channel.",
            role.station_token()
        )));
    }
    // Enforce the Freeport Registry prospecting licence (`D-35-13`).
    require_prospecting_licence(ctx.w, ctx.actor)?;

    let raw = ctx.args.raw().trim();
    let parts: Vec<&str> = raw.split_whitespace().collect();
    let rock_token = parts
        .first()
        .copied()
        .ok_or_else(|| CmdError::msg("Usage: +mine/laser <A|B|C> [<family>]"))?;

    let (session, site) = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| {
            let s = b.cluster_sessions.get(&hull)?.clone();
            let site = b.cluster_sites.get(&s.cluster)?.clone();
            Some((s, site))
        })
        .ok_or_else(|| {
            CmdError::msg("Ship is not in an orbital cluster; use `+helm/cluster <id>` first.")
        })?;
    let spec = session.cluster.rock(rock_token).ok_or_else(|| {
        CmdError::msg(format!(
            "Unknown asteroid pocket `{rock_token}` in {} (choose A, B, or C).",
            session.cluster.display_name()
        ))
    })?;

    if session.matched_rock != Some(spec.letter) {
        return Err(CmdError::msg(format!(
            "Must lock RCS velocity match (`+helm/match {}`) before firing the mining laser.",
            spec.letter
        )));
    }
    let _ = super::mining::check_velocity_match_gate(
        session.ship_pos_m,
        session.ship_vel_mmps,
        spec.pos_m,
        spec.vel_mmps,
    )
    .map_err(CmdError::msg)?;

    if session.active_laser.is_some() {
        return Err(CmdError::msg(
            "Mining laser is already running an extraction cycle on this ship.",
        ));
    }

    let family = match parts.get(1) {
        Some(tok) => {
            let fam = super::mining::parse_ore_family(tok).ok_or_else(|| {
                CmdError::msg(
                    "Unknown ore family (choose metals, volatiles, carbon_feedstock, or strategic_elements).",
                )
            })?;
            if !spec.spectral.families().contains(&fam) {
                return Err(CmdError::msg(format!(
                    "Block {} ({}) does not bear `{}`.",
                    spec.letter,
                    spec.spectral.code(),
                    fam.id()
                )));
            }
            fam
        }
        None => spec.spectral.primary_family(),
    };

    let used_t = super::mining::hull_hold_used_tonnes(ctx, hull);
    if used_t >= super::mining::DRAY_P_HOLD_CAPACITY_T {
        return Err(CmdError::msg(format!(
            "`stow.ore` hold is at capacity ({used_t}/{} t).",
            super::mining::DRAY_P_HOLD_CAPACITY_T
        )));
    }
    let rock_vault = *site
        .vaults
        .get(&spec.letter)
        .ok_or_else(|| CmdError::msg("Rock deposit vault is not laid up."))?;
    let avail_t = super::mining::rock_pool_tonnes(ctx, rock_vault, family);
    if avail_t == 0 {
        return Err(CmdError::msg(format!(
            "Block {} `{}` deposit is exhausted (0 t remaining).",
            spec.letter,
            crate::surface::opening::ore_good(family)
        )));
    }

    let now_ic = ctx.w.world_ic_now().at;
    let due_ic = now_ic.saturating_add(super::mining::LASER_CYCLE_IC_SECS);
    let yield_t = super::mining::LASER_CYCLE_YIELD_T
        .min(super::mining::DRAY_P_HOLD_CAPACITY_T.saturating_sub(used_t))
        .min(avail_t);

    // Check if this is unleased/claim-jumped frontier extraction (`doc 35` §7.4): generate
    // thermal bloom (`+18` heat) and flag claim-jump, without scripting any rival spawn inside
    // `+mine/laser` (`35a` §3 #9).
    let is_frontier_unleased = session.cluster.zone() == super::mining::ClusterZone::Frontier
        && super::mining::cluster_rock_tenure_word(ctx, session.cluster, spec, ctx.actor, now_ic)
            != format!(
                "your claim (until {}, contest window {}d)",
                crate::port::licence::ic_stamp(now_ic + crate::surface::tenure::CLAIM_TERM_IC_SECS),
                crate::surface::tenure::CONTEST_WINDOW_IC_SECS / 86_400
            );

    let mut bloom_note = String::new();
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
        && let Some(sess_mut) = book.cluster_sessions.get_mut(&hull)
    {
        sess_mut.active_laser = Some(super::mining::LaserExtractionCycle {
            rock_letter: spec.letter,
            family,
            started_ic: now_ic,
            due_ic,
            yield_t,
            operator: ctx.actor,
        });
        if is_frontier_unleased {
            sess_mut.frontier_bloom_heat = sess_mut
                .frontier_bloom_heat
                .saturating_add(super::mining::FRONTIER_LASER_BLOOM_HEAT);
            sess_mut.claim_jump_flagged = true;
            bloom_note = format!(
                " [FRONTIER BLOOM +{} heat (total {}); unleased extraction flagged]",
                super::mining::FRONTIER_LASER_BLOOM_HEAT,
                sess_mut.frontier_bloom_heat
            );
        }
    }

    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "Mining laser engaged on Block {} ({}) for {}s: extracting {yield_t} t `{}` ({} L/t) into `stow.ore`{bloom_note}.",
            spec.letter,
            spec.spectral.code(),
            super::mining::LASER_CYCLE_IC_SECS,
            crate::surface::opening::ore_good(family),
            super::mining::ore_specific_volume_l_per_t(family),
        ),
    );
    Ok(())
}

fn mine_drones(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (hull, role) = require_aboard_and_seat(ctx, None)?;
    if !role.grants_channel("flight_ops") {
        return Err(CmdError::msg(format!(
            "Your bridge station ({}) does not control the flight_ops channel.",
            role.station_token()
        )));
    }
    // Enforce the Freeport Registry prospecting licence (`D-35-13`).
    require_prospecting_licence(ctx.w, ctx.actor)?;

    // Verify the hull is a `Dray-P` (`dray_p`) prospector variant (`D-35-13`).
    let is_prospector =
        crate::port::ship::hull_view(ctx.w, hull).is_some_and(|v| v.is_prospector());
    if !is_prospector {
        return Err(CmdError::msg(
            "Collector drones require a `Dray-P` prospector hull (`+coop/refit prospector`).",
        ));
    }

    let raw = ctx.args.raw().trim().to_owned();
    let parts: Vec<&str> = raw.split_whitespace().collect();
    let rock_token = parts
        .first()
        .copied()
        .ok_or_else(|| CmdError::msg("Usage: +mine/drones <A|B|C> [<count 1..4>] [<family>]"))?;

    let mut count: usize = 4;
    let mut family_token: Option<&str> = None;
    let max_drones = crate::surface::excursion::MAX_EXCURSIONS_PER_OPERATOR as usize;
    for &arg in parts.iter().skip(1) {
        if let Ok(n) = arg.parse::<usize>() {
            if !(1..=max_drones).contains(&n) {
                return Err(CmdError::msg(
                    "Collector drone count must be between 1 and 4 (`MAX_EXCURSIONS_PER_OPERATOR`).",
                ));
            }
            count = n;
        } else {
            family_token = Some(arg);
        }
    }

    let (session, site) = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .and_then(|b| {
            let s = b.cluster_sessions.get(&hull)?.clone();
            let site = b.cluster_sites.get(&s.cluster)?.clone();
            Some((s, site))
        })
        .ok_or_else(|| {
            CmdError::msg("Ship is not in an orbital cluster; use `+helm/cluster <id>` first.")
        })?;
    let spec = session.cluster.rock(rock_token).ok_or_else(|| {
        CmdError::msg(format!(
            "Unknown asteroid pocket `{rock_token}` in {} (choose A, B, or C).",
            session.cluster.display_name()
        ))
    })?;

    if session.matched_rock != Some(spec.letter) {
        return Err(CmdError::msg(format!(
            "Must lock RCS velocity match (`+helm/match {}`) before launching collector drones.",
            spec.letter
        )));
    }
    let _ = super::mining::check_velocity_match_gate(
        session.ship_pos_m,
        session.ship_vel_mmps,
        spec.pos_m,
        spec.vel_mmps,
    )
    .map_err(CmdError::msg)?;

    let active_count = session.active_drones.len();
    if active_count + count > max_drones {
        return Err(CmdError::msg(format!(
            "Drone bay cap exceeded: {active_count} collector drone(s) already in flight (max {} under `MAX_EXCURSIONS_PER_OPERATOR`).",
            crate::surface::excursion::MAX_EXCURSIONS_PER_OPERATOR
        )));
    }

    let family = match family_token {
        Some(tok) => {
            let fam = super::mining::parse_ore_family(tok).ok_or_else(|| {
                CmdError::msg(
                    "Unknown ore family (choose metals, volatiles, carbon_feedstock, or strategic_elements).",
                )
            })?;
            if !spec.spectral.families().contains(&fam) {
                return Err(CmdError::msg(format!(
                    "Block {} ({}) does not bear `{}`.",
                    spec.letter,
                    spec.spectral.code(),
                    fam.id()
                )));
            }
            fam
        }
        None => spec.spectral.primary_family(),
    };

    let used_t = super::mining::hull_hold_used_tonnes(ctx, hull);
    if used_t >= super::mining::DRAY_P_HOLD_CAPACITY_T {
        return Err(CmdError::msg(format!(
            "`stow.ore` hold is at capacity ({used_t}/{} t).",
            super::mining::DRAY_P_HOLD_CAPACITY_T
        )));
    }
    let rock_vault = *site
        .vaults
        .get(&spec.letter)
        .ok_or_else(|| CmdError::msg("Rock deposit vault is not laid up."))?;
    let avail_t = super::mining::rock_pool_tonnes(ctx, rock_vault, family);
    if avail_t == 0 {
        return Err(CmdError::msg(format!(
            "Block {} `{}` deposit is exhausted (0 t remaining).",
            spec.letter,
            crate::surface::opening::ore_good(family)
        )));
    }

    let now_ic = ctx.w.world_ic_now().at;
    let due_ic = now_ic.saturating_add(super::mining::DRONE_SORTIE_IC_SECS);
    let mut launched_ids = Vec::new();
    for idx in 0..count {
        let slot = u8::try_from(active_count + idx + 1).unwrap_or(1);
        let drone_obj =
            super::mining::ensure_drone_thing(ctx, hull, slot).map_err(CmdError::msg)?;
        let excursion_id = crate::surface::excursion::ExcursionId {
            tv: drone_obj,
            launch_ic: now_ic,
        };
        launched_ids.push((slot, excursion_id));
    }

    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
        && let Some(sess_mut) = book.cluster_sessions.get_mut(&hull)
    {
        for &(slot, excursion_id) in &launched_ids {
            sess_mut
                .active_drones
                .push(super::mining::DroneSortieState {
                    slot,
                    excursion_id,
                    rock_letter: spec.letter,
                    family,
                    due_ic,
                    yield_t: super::mining::DRONE_SORTIE_YIELD_T,
                    operator: ctx.actor,
                });
        }
    }

    let exc_summary: Vec<String> = launched_ids
        .iter()
        .map(|(slot, exc)| format!("slot {slot} (#{exc})"))
        .collect();
    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.space",
        format!(
            "Launched {} collector drone(s) onto Block {} ({}) for {}s ({} t `{}` each into `stow.ore`): {}.",
            launched_ids.len(),
            spec.letter,
            spec.spectral.code(),
            super::mining::DRONE_SORTIE_IC_SECS,
            super::mining::DRONE_SORTIE_YIELD_T,
            crate::surface::opening::ore_good(family),
            exc_summary.join(", ")
        ),
    );
    Ok(())
}

#[allow(dead_code)]
fn _keep_bubble_import(_: &LocalSpaceBubble) {}
