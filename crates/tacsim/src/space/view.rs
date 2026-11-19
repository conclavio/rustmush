//! **One view for every space mind** (doc 44 unit MW.9; Q-44-24).
//!
//! Every mind that flies a hull inside a bubble (the customs corvette, the raider, the
//! interdictor, and the backup mind of a hull whose crew has no link) reads the view
//! [`space_mind_view`] builds, and nothing else of the world.
//!
//! # What it holds beyond the arena's view
//!
//! A [`SpaceView`], hung on [`MindView::space`] and read from a script through the kernels
//! `space(v)` and `space_contact(v, id)` (`crate::minds::kernels`):
//!
//! * her own velocity as a vector, her pitch, what her drive gives now, her reaction mass and
//!   the reserve she needs to get home, and which of the four emission postures she holds;
//! * the stop a picket or an interdictor has on her ([`Stop`]), the contact that holds it, and
//!   the furthest word a picket has spoken in the bubble. A hail and a clearance go out on
//!   open radio, so a hull that holds no contact at all still hears them;
//! * for each contact she holds: its elevation, its velocity as a vector, the posture its
//!   emissions read as, and the stop it is under.
//!
//! # Contact ids
//!
//! A contact's id is its number in the observer's own [`ContactBook`], given the first time
//! her sensors hold it and kept for the life of the bubble. A ship she has never detected has
//! no number, and takes none from the ships she has: the view of an observer is the same
//! bytes whether or not a ship she cannot detect is there.
//!
//! # The helm word
//!
//! `helm(mdeg)` carries one number. In space that number is a direction in three dimensions:
//! the bearing in millidegrees, plus a whole turn (360,000) for each tenth of a degree of
//! elevation ([`helm_word`], [`helm_direction`]). A plain bearing is a level burn, and the
//! arena, which reads a heading modulo a turn, sees the bearing alone. A script never does
//! the packing: the kernels `course(bearing_mdeg, elevation_mdeg)` and `course_of(dx, dy, dz)`
//! answer the word.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::bubble::{
    EmconPosture, FacetBand, HullStatus, LocalSpaceBubble, SpaceShipState, SubsystemCrit,
};
use super::picket::{HEAVE_TO_REL_VEL_MMPS, relative_speed_3d_mmps};
use super::weapons::{LaserMode, compute_railgun_lead, isqrt_i128};
use crate::arena::mind::{MindView, ViewContact};
use crate::arena::perception::{Aspect, FrameCondition, FrameNotice, Receiver as FrameReceiver};
use crate::arena::state::{Classification, ContactId, Designator, MatchMode, Power};
use crate::arena::step::FrameHeat;
use crate::arena::weapons::FrameWeapon;

/// One whole turn of bearing, in millidegrees: what each tenth of a degree of elevation adds
/// to a helm word.
pub const HELM_TURN_MDEG: u32 = 360_000;

/// **The stop a picket or an interdictor has on a hull**, as every mind that holds her reads
/// it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stop {
    /// Nobody is stopping her.
    #[default]
    None,
    /// She has been told to heave to and is still moving against the ship that told her.
    Hailed,
    /// She is hove to under a customs hail and has not been ruled on.
    HoveTo,
    /// She was stopped and is free to go: inspected, or stood down by an interdictor.
    Cleared,
    /// She ran a picket, or took up the fight again after she was stood down.
    Ran,
}

impl Stop {
    /// The word a script reads: `none`, `hailed`, `hove_to`, `cleared`, `ran`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Stop::None => "none",
            Stop::Hailed => "hailed",
            Stop::HoveTo => "hove_to",
            Stop::Cleared => "cleared",
            Stop::Ran => "ran",
        }
    }

    /// Whether she is still held: hailed, or hove to and waiting.
    #[must_use]
    pub const fn holds(self) -> bool {
        matches!(self, Stop::Hailed | Stop::HoveTo)
    }
}

/// The word a script reads for an emission posture: `ghost`, `passive`, `standard`, `combat`.
#[must_use]
pub const fn emcon_word(posture: EmconPosture) -> &'static str {
    match posture {
        EmconPosture::Ghost => "ghost",
        EmconPosture::Passive => "passive",
        EmconPosture::Standard => "standard",
        EmconPosture::ActiveCombat => "combat",
    }
}

/// **What a mind reads of one contact beyond the arena's row.**
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpaceContactView {
    /// Its elevation off the bubble's plane, whole degrees, up positive.
    pub elevation_deg: i32,
    /// Its velocity in the bubble's frame, cm/s.
    pub vel_cms: [i64; 3],
    /// The stop it is under.
    pub stop: Stop,
    /// The emission posture its signature reads as.
    pub emcon: EmconPosture,
    /// **The band of its hull that faces her** (doc 49 unit CC.9): the band a shot from her
    /// strikes, one of five, `top` among them. A script reads it as `Contact.aspect`.
    pub facet: FacetBand,
    /// **How fast it and she close**, cm/s along the line between them (doc 48e kernel G8):
    /// positive while the range falls, negative while it opens; nought at no range.
    pub closing_cms: i64,
}

/// **The closing speed of a contact at `offset_m` from her**, cm/s (doc 48e kernel G8): minus
/// the relative velocity along the line from her to it, so positive while the range falls.
/// Integer throughout; nought at no range.
#[must_use]
pub fn closing_cms(offset_m: [i128; 3], own_vel_mmps: [i64; 3], other_vel_mmps: [i64; 3]) -> i64 {
    let range2 = offset_m[0] * offset_m[0] + offset_m[1] * offset_m[1] + offset_m[2] * offset_m[2];
    let range = isqrt_i128(range2);
    if range == 0 {
        return 0;
    }
    let rel = |axis: usize| i128::from(other_vel_mmps[axis]) - i128::from(own_vel_mmps[axis]);
    let along_mmps = (rel(0) * offset_m[0] + rel(1) * offset_m[1] + rel(2) * offset_m[2]) / range;
    i64::try_from(-along_mmps / 10).unwrap_or(0)
}

/// **What a mind reads of its own hull in space beyond the arena's view.**
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceView {
    /// Her velocity in the bubble's frame, cm/s.
    pub vel_cms: [i64; 3],
    /// Her nose's pitch, millidegrees, up positive.
    pub pitch_mdeg: i32,
    /// What her drive gives now, cm/s²: the most a burn of hers can be.
    pub accel_cms2: i64,
    /// Her reaction mass, kilograms.
    pub propellant_kg: u32,
    /// The change of speed her tank still holds, m/s.
    pub dv_mps: i64,
    /// The change of speed she must keep to make an open port, m/s.
    pub reserve_mps: i64,
    /// Her emission posture.
    pub emcon: EmconPosture,
    /// How long, in milliseconds, the bubble flies what she asks before she thinks again.
    pub step_ms: u32,
    /// The stop she is under.
    pub stop: Stop,
    /// The contact that holds the stop on her, when her sensors hold it.
    pub stop_by: Option<ContactId>,
    /// The furthest word a picket has spoken on open radio in this bubble, to her or to
    /// anyone: a hull that holds no contact still hears it.
    pub heard: Stop,
    /// Each held contact's row, by the id the arena's row carries.
    pub contacts: BTreeMap<ContactId, SpaceContactView>,
    /// **Every mount she carries** (doc 49 unit CC.9), point defence among them, as
    /// [`fire_picture`] reads them: what a script reads through `space_mount(v, id)`.
    pub mounts: Vec<MountFacts>,
}

/// **One observer's contact numbers** (the file head): the number each ship was given the
/// first time this observer's sensors held it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactBook {
    /// The last number given.
    #[serde(default)]
    last: u64,
    /// The number each detected ship holds, by its id in the bubble.
    #[serde(default)]
    ids: BTreeMap<String, u64>,
}

impl ContactBook {
    /// Whether no ship has been numbered: what a persisted record leaves out.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The number `ship_id` holds, giving it the next one if it has none.
    pub fn number(&mut self, ship_id: &str) -> ContactId {
        if let Some(&held) = self.ids.get(ship_id) {
            return ContactId(held);
        }
        self.last = self.last.saturating_add(1);
        self.ids.insert(ship_id.to_owned(), self.last);
        ContactId(self.last)
    }

    /// The number `ship_id` holds, if it was ever detected.
    #[must_use]
    pub fn held(&self, ship_id: &str) -> Option<ContactId> {
        self.ids.get(ship_id).copied().map(ContactId)
    }
}

/// **What the host knows of a bubble that its ships do not carry**: the stops, who holds
/// them, the step, and the posture each ship's player left on her.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Situation {
    /// How long the bubble flies before the minds think again, milliseconds.
    pub step_ms: u32,
    /// The stop each stopped ship is under, by its id in the bubble.
    pub stops: BTreeMap<String, Stop>,
    /// The ship that holds those stops: the customs corvette, or the interdictor.
    pub stopper: Option<String>,
    /// **The posture each ship holds**, by her id in the bubble (doc 45 unit AP.3; Q-45-23):
    /// read off the hull's own record (`crate::posture::record`) by the host that flies her,
    /// and `RETURN` for a ship with no row. The same word a unit on the ground reads.
    pub postures: BTreeMap<String, crate::posture::Posture>,
    /// **The ships each hull that only answers fire may answer** (doc 45 unit AP.9; Q-44-38,
    /// Q-45-8), by the hull's id in the bubble: a row for a hull flown on `HOLD` or `GUARD`
    /// for an absent player, laid by the backup seat. Her view shows as hostile only the
    /// contacts in her row; every other contact her party holds hostile reads neutral, as a
    /// unit's does on the ground (`crate::arena::absent`). A hull with no row reads every
    /// relation as her party holds it. The view is narrowed by this and never widened, and
    /// the seat's own doors do not depend on it.
    pub answerable: BTreeMap<String, std::collections::BTreeSet<String>>,
}

impl Situation {
    /// A bubble where nobody is stopping anybody, stepped `step_ms` at a time.
    #[must_use]
    pub fn open(step_ms: u32) -> Situation {
        Situation {
            step_ms,
            stops: BTreeMap::new(),
            stopper: None,
            postures: BTreeMap::new(),
            answerable: BTreeMap::new(),
        }
    }

    /// The posture `ship_id` holds: `RETURN` unless the host laid another.
    #[must_use]
    pub fn posture_of(&self, ship_id: &str) -> crate::posture::Posture {
        self.postures.get(ship_id).copied().unwrap_or_default()
    }

    /// The stop `ship_id` is under.
    #[must_use]
    pub fn stop_of(&self, ship_id: &str) -> Stop {
        self.stops.get(ship_id).copied().unwrap_or_default()
    }

    /// The furthest word spoken to any ship: what every hull in the bubble hears.
    #[must_use]
    pub fn heard(&self) -> Stop {
        self.stops.values().copied().max().unwrap_or_default()
    }
}

/// **The helm word for a direction** (the file head): `bearing_mdeg` taken modulo a turn,
/// and `elevation_mdeg` rounded to a tenth of a degree and held to straight up or down.
#[must_use]
pub fn helm_word(bearing_mdeg: i64, elevation_mdeg: i64) -> u32 {
    let bearing = u32::try_from(bearing_mdeg.rem_euclid(i64::from(HELM_TURN_MDEG))).unwrap_or(0);
    let tenths = (elevation_mdeg.clamp(-90_000, 90_000) + elevation_mdeg.signum() * 50) / 100;
    let turns = match tenths {
        0 => 0,
        up @ 1..=900 => u32::try_from(up).unwrap_or(0),
        down => 900 + u32::try_from(-down).unwrap_or(0).min(900),
    };
    bearing + turns * HELM_TURN_MDEG
}

/// **The bearing and elevation a helm word names**, in millidegrees: the inverse of
/// [`helm_word`]. A word past straight down reads as a level bearing.
#[must_use]
pub fn helm_angles(word: u32) -> (i64, i64) {
    let bearing = i64::from(word % HELM_TURN_MDEG);
    let elevation = match word / HELM_TURN_MDEG {
        up @ 1..=900 => i64::from(up) * 100,
        down @ 901..=1_800 => -i64::from(down - 900) * 100,
        _ => 0,
    };
    (bearing, elevation)
}

/// **The direction a helm word names**, as a vector of length `length` in the bubble's frame.
/// Bearing nought is the bubble's +x and a quarter turn is its +y, as a contact's
/// `bearing_deg` is read; elevation is toward +z.
#[must_use]
pub fn helm_direction(word: u32, length: i64) -> [i64; 3] {
    let (bearing_mdeg, elevation_mdeg) = helm_angles(word);
    let bearing = (bearing_mdeg as f64 / 1_000.0).to_radians();
    let elevation = (elevation_mdeg as f64 / 1_000.0).to_radians();
    let level = libm::cos(elevation);
    let scale = length as f64;
    [
        libm::round(scale * level * libm::cos(bearing)) as i64,
        libm::round(scale * level * libm::sin(bearing)) as i64,
        libm::round(scale * libm::sin(elevation)) as i64,
    ]
}

/// **The helm word for the direction of a vector** in the bubble's frame; a vector of no
/// length answers a level bearing of nought.
#[must_use]
pub fn helm_word_of(vector: [i64; 3]) -> u32 {
    let (bearing_mdeg, elevation_mdeg) = angles_of(vector);
    helm_word(bearing_mdeg, elevation_mdeg)
}

/// The bearing and elevation of `vector`, millidegrees.
fn angles_of(vector: [i64; 3]) -> (i64, i64) {
    let [x, y, z] = vector.map(|part| part as f64);
    if x == 0.0 && y == 0.0 && z == 0.0 {
        return (0, 0);
    }
    let bearing = libm::atan2(y, x).to_degrees();
    let elevation = libm::atan2(z, libm::sqrt(x * x + y * y)).to_degrees();
    (
        (libm::round(bearing * 1_000.0) as i64).rem_euclid(i64::from(HELM_TURN_MDEG)),
        libm::round(elevation * 1_000.0) as i64,
    )
}

/// What `ship`'s drive gives now, mm/s²: her rating, halved by a degraded drive.
#[must_use]
pub fn drive_mmps2(ship: &SpaceShipState) -> i64 {
    let rated = if ship.max_accel_mmps2 > 0 {
        ship.max_accel_mmps2
    } else {
        ship.max_accel_mps2.saturating_mul(1_000).max(1)
    };
    if ship.crits.contains(&SubsystemCrit::DriveDegraded) {
        (rated / 2).max(1)
    } else {
        rated
    }
}

fn cms(vel_mmps: [i64; 3]) -> [i64; 3] {
    vel_mmps.map(|part| part / 10)
}

/// What a laser's row reads as its rounds and their maximum: a laser keeps no magazine, so its
/// row reads full.
pub const LASER_ROW_ROUNDS: u32 = 100;

// ---------------------------------------------------------------------------
// Doc 49 unit CC.9: what her mounts and her lock read before a shot
// ---------------------------------------------------------------------------

/// The most a hull's heat may reach, per mille: a shot that would take her past it is not
/// taken (`super::bubble::LocalSpaceBubble::fire_offensive_lasers`, `fire_railgun_from`).
pub const HEAT_CEILING_PERMILLE: u16 = 1_000;

/// **What a mount is** (doc 49 unit CC.9): the word the cockpit and a mind read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MountFamily {
    /// A laser turret: a pulse that lands at once, its damage falling with range.
    Laser,
    /// Torpedo tubes: a salvo of flown rounds.
    Tubes,
    /// A spinal gun: one flown round, unguided.
    Spinal,
    /// A point-defence rotary: bursts that stop incoming torpedoes, on their own.
    PointDefence,
}

impl MountFamily {
    /// The word: `laser`, `tubes`, `spinal`, `point_defence`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            MountFamily::Laser => "laser",
            MountFamily::Tubes => "tubes",
            MountFamily::Spinal => "spinal",
            MountFamily::PointDefence => "point_defence",
        }
    }
}

/// **Whether a mount fires now, or the first thing that holds it back** (doc 49 unit CC.9).
/// Read in the order the variants are written; the first that holds is the mount's state.
/// Space has no chance roll (Q-49-13, as corrected by doc 48b's W-49-5): a mount that reads
/// [`MountState::Ready`] fires when it is told to, and an offence laser that fires lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MountState {
    /// Out of action: a point-defence rotary that damage has put down, or any mount of a hull
    /// that is no longer operational.
    Down,
    /// No rounds left, or too few for one burst.
    Empty,
    /// Its cycle has not finished.
    Cycling,
    /// One more shot would take her heat past [`HEAT_CEILING_PERMILLE`].
    TooHot,
    /// Her lock lies past its reach.
    PastReach,
    /// It fires now.
    Ready,
}

impl MountState {
    /// The word: `down`, `empty`, `cycling`, `too_hot`, `past_reach`, `ready`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            MountState::Down => "down",
            MountState::Empty => "empty",
            MountState::Cycling => "cycling",
            MountState::TooHot => "too_hot",
            MountState::PastReach => "past_reach",
            MountState::Ready => "ready",
        }
    }
}

/// **What a mount does to her lock at the lock's range** (doc 49 unit CC.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtLock {
    /// A laser her fit lets fire at a lock: the damage one pulse lands at that range.
    Damage(u32),
    /// A spinal gun: how long its round flies to her lock, milliseconds.
    FlightMs(u32),
}

/// **One mount, as the cockpit and a mind read it** (doc 49 unit CC.9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountFacts {
    /// Its id on the hull: what the commands and a mind's `fire` name.
    pub id: SmolStr,
    /// What it is.
    pub family: MountFamily,
    /// A laser's mode; `None` for every other mount.
    pub mode: Option<LaserMode>,
    /// Whether her fit lets it fire at a lock: every laser her fit allows offence, every tube
    /// family and every spinal gun. Point defence fires on its own.
    pub fires_at_lock: bool,
    /// Rounds left; `None` for a laser, which keeps no magazine.
    pub rounds: Option<u32>,
    /// What her fit stows: the most the magazine holds. `None` for a laser, and for a
    /// point-defence rotary, whose drum the kernel does not count back to full.
    pub rounds_max: Option<u32>,
    /// How long until its cycle has finished, milliseconds; nought when it has.
    pub ready_in_ms: u32,
    /// Whether it fires now, or what holds it back.
    pub state: MountState,
    /// How far it reaches, metres: a laser's and a spinal gun's reach, a point-defence
    /// rotary's envelope. `None` for the tubes: a torpedo flies until it reaches her or its
    /// flight ends.
    pub reach_m: Option<i64>,
    /// What one shot adds to her heat, per mille of [`HEAT_CEILING_PERMILLE`].
    pub heat_per_shot: u16,
    /// What it does to her lock at the lock's range, when her sensors hold the lock and it
    /// lies within the mount's reach; `None` for the tubes and point defence.
    pub at_lock: Option<AtLock>,
}

impl MountFacts {
    /// **Whether its cycle has finished and it has what it needs to fire at a lock**: rounds
    /// left, and a laser in offence. What a mind's row of `v.weapons` reads as `ready`, as the
    /// ground's does; heat and reach are read on their own ([`Self::state`]).
    #[must_use]
    pub fn cycled(&self) -> bool {
        self.ready_in_ms == 0
            && !matches!(self.state, MountState::Down | MountState::Empty)
            && self.mode != Some(LaserMode::Defense)
    }
}

/// **Her lock, as her sensors hold it** (doc 49 unit CC.9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockFacts {
    /// The locked ship's id in the bubble: what `+tac/lock` names.
    pub ship_id: String,
    /// The name it answers to.
    pub callsign: String,
    /// How far off it is, metres.
    pub range_m: u64,
    /// The band of her hull that faces the observer: the band a shot from here strikes.
    pub facet: FacetBand,
}

/// **What a hull reads of her mounts and her lock before a shot** (doc 49 unit CC.9): one
/// reading, which `+tac/status`, the cockpit feed and the space mind's view all print or read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FirePicture {
    /// Every mount she carries: her lasers, her tube families, her spinal guns and her
    /// point-defence rotaries, each family in her fit's order.
    pub mounts: Vec<MountFacts>,
    /// The ship her lock names, whether or not her sensors still hold it.
    pub lock_id: Option<String>,
    /// The locked ship, when her sensors hold it.
    pub lock: Option<LockFacts>,
}

/// The distance from `from` to `to`, whole metres, as the bubble's fire measures it.
fn distance_m(from: [i64; 3], to: [i64; 3]) -> u64 {
    let d = [0, 1, 2].map(|axis| i128::from(to[axis]) - i128::from(from[axis]));
    u64::try_from(isqrt_i128(d[0] * d[0] + d[1] * d[1] + d[2] * d[2])).unwrap_or(u64::MAX)
}

/// **The fire picture of `ship_id` in `bubble`** (doc 49 unit CC.9), or `None` for a ship the
/// bubble does not hold. Every figure is the kernel's own: the cycle left, the magazine and
/// what her fit stows, the laser's mode, the heat refusal, a laser's damage at range
/// ([`super::weapons::LaserMount::damage_at_range`]), a spinal round's flight
/// ([`compute_railgun_lead`]) and the band of the target's hull a shot from here strikes
/// ([`SpaceShipState::select_facet_for_attack`]).
#[must_use]
pub fn fire_picture(bubble: &LocalSpaceBubble, ship_id: &str) -> Option<FirePicture> {
    let ship = bubble.ships.get(ship_id)?;
    let target = ship
        .locked_target_id
        .as_deref()
        .and_then(|id| bubble.ships.get(id))
        .filter(|target| {
            target.id != ship.id
                && super::commands::is_bubble_ship_detected(ship, target, bubble.ic_timestamp_s)
        });
    Some(ship_picture(ship, target))
}

/// [`fire_picture`] of `ship`, whose lock her sensors hold as `target` (`None` with no lock
/// held).
fn ship_picture(ship: &SpaceShipState, target: Option<&SpaceShipState>) -> FirePicture {
    let lock_id = ship.locked_target_id.clone();
    let lock = target.map(|target| LockFacts {
        ship_id: target.id.clone(),
        callsign: target.callsign.clone(),
        range_m: distance_m(ship.pos_m, target.pos_m),
        facet: target.select_facet_for_attack(ship.pos_m),
    });
    let range_m = lock
        .as_ref()
        .map(|lock| i64::try_from(lock.range_m).unwrap_or(i64::MAX));
    let too_hot = |heat: u16| ship.heat_permille.saturating_add(heat) > HEAT_CEILING_PERMILLE;

    let mut mounts = Vec::new();
    for laser in &ship.lasers {
        let damage = range_m
            .filter(|_| laser.offence_allowed())
            .map(|range| laser.damage_at_range(range));
        // Doc 48e kernel G5: a laser too hot to fire reads so in either mode (the kernel
        // refuses a hot defence shot too), and one in defence reaches its defence range.
        let state = if laser.cooldown_remaining_ms > 0 {
            MountState::Cycling
        } else if too_hot(laser.heat_cost_permille) {
            MountState::TooHot
        } else if laser.mode == LaserMode::Defense {
            MountState::Ready
        } else if damage == Some(0) {
            MountState::PastReach
        } else {
            MountState::Ready
        };
        mounts.push(MountFacts {
            id: SmolStr::new(&laser.id),
            family: MountFamily::Laser,
            mode: Some(laser.mode),
            fires_at_lock: laser.offence_allowed(),
            rounds: None,
            rounds_max: None,
            ready_in_ms: laser.cooldown_remaining_ms,
            state,
            reach_m: Some(if laser.mode == LaserMode::Defense {
                laser.defense_range_m
            } else {
                laser.max_range_m
            }),
            heat_per_shot: laser.heat_cost_permille,
            at_lock: damage.filter(|damage| *damage > 0).map(AtLock::Damage),
        });
    }
    for tube in &ship.torpedo_tubes {
        let state = if tube.magazine_remaining == 0 {
            MountState::Empty
        } else if tube.cooldown_remaining_ms > 0 {
            MountState::Cycling
        } else {
            MountState::Ready
        };
        mounts.push(MountFacts {
            id: SmolStr::new(&tube.id),
            family: MountFamily::Tubes,
            mode: None,
            fires_at_lock: true,
            rounds: Some(u32::from(tube.magazine_remaining)),
            rounds_max: Some(u32::from(tube.magazine_max())),
            ready_in_ms: tube.cooldown_remaining_ms,
            state,
            reach_m: None,
            heat_per_shot: 0,
            at_lock: None,
        });
    }
    for gun in &ship.railguns {
        let in_reach = range_m.map(|range| range <= gun.max_range_m);
        let state = if gun.rounds_remaining == 0 {
            MountState::Empty
        } else if gun.cooldown_remaining_ms > 0 {
            MountState::Cycling
        } else if too_hot(gun.heat_cost_permille) {
            MountState::TooHot
        } else if in_reach == Some(false) {
            MountState::PastReach
        } else {
            MountState::Ready
        };
        let flight = target.filter(|_| in_reach == Some(true)).map(|target| {
            compute_railgun_lead(
                ship.pos_m,
                ship.vel_mmps,
                target.pos_m,
                target.vel_mmps,
                gun.muzzle_vel_mps,
            )
            .1
        });
        mounts.push(MountFacts {
            id: SmolStr::new(&gun.id),
            family: MountFamily::Spinal,
            mode: None,
            fires_at_lock: true,
            rounds: Some(u32::from(gun.rounds_remaining)),
            rounds_max: Some(u32::from(gun.magazine_max())),
            ready_in_ms: gun.cooldown_remaining_ms,
            state,
            reach_m: Some(gun.max_range_m),
            heat_per_shot: gun.heat_cost_permille,
            at_lock: flight.map(AtLock::FlightMs),
        });
    }
    for pd in &ship.pdcs {
        let state = if !pd.operational {
            MountState::Down
        } else if pd.ammo_remaining < u32::from(pd.burst_rounds) {
            MountState::Empty
        } else if pd.cooldown_remaining_ms > 0 {
            MountState::Cycling
        } else {
            MountState::Ready
        };
        mounts.push(MountFacts {
            id: SmolStr::new(&pd.id),
            family: MountFamily::PointDefence,
            mode: None,
            fires_at_lock: false,
            rounds: Some(pd.ammo_remaining),
            rounds_max: None,
            ready_in_ms: pd.cooldown_remaining_ms,
            state,
            reach_m: Some(pd.envelope_m),
            heat_per_shot: 0,
            at_lock: None,
        });
    }
    // A hull that is not operational fires nothing (every fire path refuses her): each of her
    // mounts is out of action.
    if ship.status != HullStatus::Operational {
        for mount in &mut mounts {
            mount.state = MountState::Down;
        }
    }
    FirePicture {
        mounts,
        lock_id,
        lock,
    }
}

/// **The fire picture as the `space.cockpit` feed's `fire` object** (doc 49 unit CC.9): what
/// `mush_pixel::space_cockpit::fire_rows` prints, on the canopy, in the TUI's space panel and
/// in `+tac/status`. No field carries odds: space has no chance roll.
#[must_use]
pub fn fire_feed(picture: &FirePicture) -> serde_json::Value {
    let mounts: Vec<serde_json::Value> = picture
        .mounts
        .iter()
        .map(|mount| {
            let mut row = serde_json::json!({
                "id": mount.id.as_str(),
                "family": mount.family.as_str(),
                "ready_in_ms": mount.ready_in_ms,
                "state": mount.state.as_str(),
            });
            if let Some(mode) = mount.mode {
                row["mode"] = serde_json::json!(laser_mode_word(mode));
            }
            if let Some(left) = mount.rounds {
                row["rounds"] = serde_json::json!(left);
            }
            if let Some(max) = mount.rounds_max {
                row["rounds_max"] = serde_json::json!(max);
            }
            match mount.at_lock {
                Some(AtLock::Damage(damage)) => row["damage"] = serde_json::json!(damage),
                Some(AtLock::FlightMs(ms)) => row["flight_ms"] = serde_json::json!(ms),
                None => {}
            }
            row
        })
        .collect();
    let lock = picture.lock.as_ref().map(|lock| {
        serde_json::json!({
            "id": lock.ship_id,
            "label": lock.callsign,
            "range_m": lock.range_m,
            "facet": lock.facet.id_str(),
        })
    });
    serde_json::json!({
        "mounts": mounts,
        "lock_id": picture.lock_id,
        "lock": lock,
    })
}

/// A laser's mode as the cockpit and a mind read it: `offence` or `defence`.
#[must_use]
pub const fn laser_mode_word(mode: LaserMode) -> &'static str {
    match mode {
        LaserMode::Offense => "offence",
        LaserMode::Defense => "defence",
    }
}

/// **The mounts a mind may fire, as the rows a script reads them by** (doc 49 unit CC.6: one row
/// a mount, each fired by its id): every laser her fit lets fire at her lock (Q-49-24), then
/// every tube family, then every spinal gun, each family in her fit's order; then a drone's
/// collector. A magazine's maximum is what her fit stows ([`super::weapons::TorpedoTube::magazine_max`]),
/// so it does not shrink as she fires. Point defence fires on its own and has no row.
///
/// **Doc 49 unit CC.9: every figure is the kernel's own** ([`fire_picture`]). `ready` is
/// [`MountFacts::cycled`]: a laser in defence is not ready to fire at a lock. There are no odds
/// (`p_permille` is `None`): a mount that is ready, under her heat ceiling and within reach
/// fires, and an offence laser that fires lands. `bands_cm` is the mount's one reach (none for
/// the tubes), and `heat_per_shot` what a shot adds to her heat, per mille of the ceiling that
/// [`SpaceView::mounts`]' `too_hot` reads.
fn weapon_rows(observer: &SpaceShipState, picture: &FirePicture) -> Vec<FrameWeapon> {
    let mut weapons: Vec<FrameWeapon> = picture
        .mounts
        .iter()
        .filter(|mount| mount.fires_at_lock)
        .map(|mount| {
            let rounds = mount.rounds.unwrap_or(LASER_ROW_ROUNDS);
            let rounds_max = mount.rounds_max.unwrap_or(LASER_ROW_ROUNDS);
            FrameWeapon {
                id: mount.id.clone(),
                ready: mount.cycled(),
                ready_in_ms: i64::from(mount.ready_in_ms),
                rounds,
                rounds_max,
                range_band: None,
                p_permille: None,
                bands_cm: mount
                    .reach_m
                    .map(|reach| {
                        u32::try_from(reach.saturating_mul(100).max(0)).unwrap_or(u32::MAX)
                    })
                    .into_iter()
                    .collect(),
                heat_per_shot: u32::from(mount.heat_per_shot),
                armed: None,
            }
        })
        .collect();
    // Doc 44 unit MW.12 (Q-44-27): a collector drone's one mount is its collector, and the
    // row is how its mind reads the hold. `rounds` is the lifts the hold still has room
    // for and `rounds_max` the lifts a whole hold takes; it is ready when it has cycled and
    // has room; its one band is its reach.
    if let Some(kit) = &observer.collector {
        weapons.push(FrameWeapon {
            id: SmolStr::new(&kit.id),
            ready: kit.ready(),
            ready_in_ms: i64::from(kit.cooldown_remaining_ms),
            rounds: kit.lifts_left(),
            rounds_max: kit.hold_cap_kg / super::bubble::COLLECTOR_LIFT_KG,
            range_band: None,
            // Doc 49 unit CC.9: no odds anywhere in space; a lift that is ready lands.
            p_permille: None,
            bands_cm: vec![
                u32::try_from(super::bubble::COLLECTOR_REACH_M.saturating_mul(100))
                    .unwrap_or(u32::MAX),
            ],
            heat_per_shot: 0,
            armed: None,
        });
    }
    weapons
}

/// **The arena's word for a space facet band** (doc 49 unit CC.9). The arena's four sectors
/// cannot say `top`, so a top band reads `front` here. Nothing in space reads this field: a
/// script reads the five bands through `Contact.aspect`, which answers [`SpaceContactView::facet`]
/// in a space view.
const fn ground_aspect(facet: FacetBand) -> Aspect {
    match facet {
        FacetBand::Front | FacetBand::Top => Aspect::Front,
        FacetBand::Left => Aspect::Left,
        FacetBand::Right => Aspect::Right,
        FacetBand::Rear => Aspect::Rear,
    }
}

/// **The view `ship_id` reads inside `bubble`** (the file head), and the ship each of its
/// contact rows names. `book` is the observer's own and is given a number for each ship her
/// sensors hold for the first time; `situation` is what the host knows of the stops.
pub fn space_mind_view(
    bubble: &LocalSpaceBubble,
    ship_id: &str,
    unit_obj: ObjId,
    book: &mut ContactBook,
    situation: &Situation,
) -> Option<(MindView, BTreeMap<ContactId, String>)> {
    let observer = bubble.ships.get(ship_id)?;
    let picture = fire_picture(bubble, ship_id)?;

    let mut contact_map = BTreeMap::new();
    let mut contacts = Vec::new();
    let mut rows = BTreeMap::new();
    for other in bubble.ships.values() {
        if other.id == ship_id
            || !super::commands::is_bubble_ship_detected(observer, other, bubble.ic_timestamp_s)
        {
            continue;
        }
        // Numbered here, after the detection filter: an undetected ship takes no number.
        let cid = book.number(&other.id);
        contact_map.insert(cid, other.id.clone());

        let rel = bubble.party_relation(observer.party, other.party);
        // Doc 45 unit AP.9: a hull that only answers fire is shown as hostile only what
        // she may answer.
        let rel = match situation.answerable.get(ship_id) {
            Some(may)
                if rel == crate::arena::party::Relation::Hostile && !may.contains(&other.id) =>
            {
                crate::arena::party::Relation::Neutral
            }
            _ => rel,
        };
        let rel_speed_mmps = relative_speed_3d_mmps(observer.vel_mmps, other.vel_mmps);
        let offset = [
            i128::from(other.pos_m[0]) - i128::from(observer.pos_m[0]),
            i128::from(other.pos_m[1]) - i128::from(observer.pos_m[1]),
            i128::from(other.pos_m[2]) - i128::from(observer.pos_m[2]),
        ];
        let range_m = u64::try_from(isqrt_i128(
            offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2],
        ))
        .unwrap_or(u64::MAX);
        let (bearing_mdeg, elevation_mdeg) =
            angles_of(offset.map(|part| i64::try_from(part).unwrap_or(0)));
        let bearing_deg = (libm::round(bearing_mdeg as f64 / 1_000.0) as i32).rem_euclid(360);
        let letter = char::from(b'A' + u8::try_from((cid.0.saturating_sub(1)) % 26).unwrap_or(0));
        let band = if range_m <= 15_000 {
            SmolStr::new_static("near")
        } else if range_m <= 60_000 {
            SmolStr::new_static("medium")
        } else {
            SmolStr::new_static("far")
        };
        // Doc 49 unit CC.9: the band of her hull a shot from here strikes, of five.
        let facet = other.select_facet_for_attack(observer.pos_m);
        // Doc 48e kernel G8: the closing speed, along the line between them.
        let closing_cms = closing_cms(offset, observer.vel_mmps, other.vel_mmps);
        rows.insert(
            cid,
            SpaceContactView {
                elevation_deg: libm::round(elevation_mdeg as f64 / 1_000.0) as i32,
                vel_cms: cms(other.vel_mmps),
                stop: situation.stop_of(&other.id),
                emcon: other.emcon,
                facet,
                closing_cms,
            },
        );
        contacts.push(ViewContact {
            id: cid,
            designator: Designator::Unit {
                party: other.party,
                letter,
            },
            alias: None,
            class: Classification::Classified { party: other.party },
            friendly: rel.is_friendly(),
            relation: rel,
            moving: rel_speed_mmps >= HEAVE_TO_REL_VEL_MMPS,
            bearing_deg: bearing_deg as u32,
            relative_deg: (bearing_deg - observer.attitude_deg.0).rem_euclid(360) as u32,
            range_m,
            band,
            age_ms: 0,
            cell: Some((8, 8)),
            condition: None::<FrameCondition>,
            fatal: Some(Vec::new()),
            aspect: ground_aspect(facet),
            // Space has no chance roll, so no row carries odds (Q-49-13 as corrected).
            odds: Vec::new(),
            // Doc 44 unit MW.12: a rock that bears no ore reads as a hull that is done.
            reads_dead: Some(other.status != HullStatus::Operational || other.is_spent_rock()),
        });
    }
    contacts.sort_by_key(|c| (c.range_m, c.id));

    let own_stop = situation.stop_of(ship_id);
    let space = SpaceView {
        vel_cms: cms(observer.vel_mmps),
        pitch_mdeg: observer.attitude_deg.1.saturating_mul(1_000),
        accel_cms2: drive_mmps2(observer) / 10,
        propellant_kg: observer.propellant_kg,
        dv_mps: observer.delta_v_mps,
        reserve_mps: observer.return_reserve_dv_mps,
        emcon: observer.emcon,
        step_ms: situation.step_ms,
        stop: own_stop,
        stop_by: situation
            .stopper
            .as_deref()
            .filter(|_| own_stop != Stop::None)
            .and_then(|stopper| book.held(stopper))
            .filter(|id| contact_map.contains_key(id)),
        heard: situation.heard(),
        contacts: rows,
        mounts: picture.mounts.clone(),
    };

    let t_ms = i64::try_from(bubble.wall_elapsed_ms).unwrap_or(i64::MAX);
    let view = MindView {
        unit: unit_obj.0,
        callsign: Designator::Unit {
            party: observer.party,
            letter: 'A',
        },
        party: observer.party,
        cell: Some((8, 8)),
        mode: MatchMode::Skirmish,
        own_last_hit_ms: None,
        objective: None,
        motor: None,
        x_cm: observer.pos_m[0].saturating_mul(100),
        y_cm: observer.pos_m[1].saturating_mul(100),
        z_cm: observer.pos_m[2].saturating_mul(100),
        heading_mdeg: (observer.attitude_deg.0.rem_euclid(360) as u32) * 1_000,
        speed_cms: (isqrt_i128(
            i128::from(observer.vel_mmps[0]).pow(2)
                + i128::from(observer.vel_mmps[1]).pow(2)
                + i128::from(observer.vel_mmps[2]).pow(2),
        ) / 10) as i32,
        power: Power::Online,
        // Doc 35 §11 `B-8`: true while a stop holds her.
        held: own_stop.holds(),
        wreck: observer.status != HullStatus::Operational,
        immobile: observer.status != HullStatus::Operational,
        sight_m: 250_000,
        ecm: Some(observer.active_decoy_ms > 0),
        receiver: Some(FrameReceiver {
            budget: 100,
            budget_max: 100,
            reach_permille: 1_000,
        }),
        heat: Some(FrameHeat {
            heat: u32::from(observer.heat_permille),
            max: 1_000,
        }),
        sections: Vec::new(),
        // Her own lock, when her sensors still hold it: never a contact picked for her.
        lock: observer
            .locked_target_id
            .as_deref()
            .and_then(|locked| book.held(locked))
            .filter(|id| contact_map.contains_key(id)),
        weapons: weapon_rows(observer, &picture),
        rules: Vec::new(),
        order: None,
        t_ms,
        notices: Vec::<FrameNotice>::new(),
        contacts,
        scan_tables: Vec::new(),
        odds_floor_permille: 0,
        space: Some(Arc::new(space)),
        // Doc 45 unit AP.3: the posture her player left on her, the ground's own word.
        posture: situation.posture_of(ship_id),
    };
    Some((view, contact_map))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Doc 48e kernel G8: the closing speed** is along the line between them, positive while
    /// the range falls: a hull running straight away at 80 m/s opens at 80 m/s, one crossing
    /// her line closes at nothing, and one coming at her closes.
    #[test]
    fn g8_the_closing_speed_is_along_the_line_between_them() {
        let ahead = [20_000_i128, 0, 0];
        assert_eq!(closing_cms(ahead, [0, 0, 0], [80_000, 0, 0]), -8_000);
        assert_eq!(closing_cms(ahead, [0, 0, 0], [0, 80_000, 0]), 0);
        assert_eq!(closing_cms(ahead, [0, 0, 0], [-30_000, 0, 0]), 3_000);
        assert_eq!(closing_cms(ahead, [50_000, 0, 0], [80_000, 0, 0]), -3_000);
        assert_eq!(closing_cms([3, 4, 0], [0, 0, 0], [-3_000, -4_000, 0]), 500);
        assert_eq!(closing_cms([0, 0, 0], [0, 0, 0], [1, 1, 1]), 0);
    }

    #[test]
    fn a_helm_word_keeps_its_bearing_and_its_elevation_to_a_tenth_of_a_degree() {
        for (bearing, elevation) in [
            (0, 0),
            (359_999, 0),
            (90_000, 45_000),
            (180_000, -30_500),
            (12_345, 90_000),
            (12_345, -90_000),
            (7, 100),
            (7, -100),
        ] {
            let word = helm_word(bearing, elevation);
            assert_eq!(
                helm_angles(word),
                (bearing, elevation),
                "{bearing} {elevation}"
            );
        }
        // A plain bearing is a level burn, and the arena's modulo reads the bearing alone.
        assert_eq!(helm_word(123_000, 0), 123_000);
        assert_eq!(helm_word(123_000, 45_000) % HELM_TURN_MDEG, 123_000);
        // Past straight up is held there.
        assert_eq!(helm_angles(helm_word(0, 200_000)), (0, 90_000));
    }

    #[test]
    fn a_helm_word_names_the_direction_of_the_vector_it_was_made_from() {
        for vector in [
            [1_000, 0, 0],
            [0, 1_000, 0],
            [0, 0, 1_000],
            [0, 0, -1_000],
            [-700, 700, 0],
            [300, -400, 1_200],
        ] {
            let back = helm_direction(helm_word_of(vector), 10_000);
            let length = isqrt_i128(vector.iter().map(|p| i128::from(*p).pow(2)).sum()) as i64;
            for axis in 0..3 {
                let want = vector[axis] * 10_000 / length;
                assert!(
                    (back[axis] - want).abs() <= 20,
                    "{vector:?} axis {axis}: {} against {want}",
                    back[axis]
                );
            }
        }
        assert_eq!(helm_word_of([0, 0, 0]), 0);
    }

    /// Doc 45 unit AP.3: a ship's view carries the posture the host laid for her, by her own id
    /// in the bubble, and `RETURN` for every ship with no row. One ship's word is not another's.
    #[test]
    fn a_ships_view_carries_her_own_posture_and_return_without_one() {
        use crate::posture::Posture;
        use crate::space::picket::{CARRIER_PARTY, CUSTOMS_CORVETTE_PARTY};

        let mut bubble = LocalSpaceBubble::new("bubble.test.posture", "domain.4", "Test", 1_000);
        bubble.insert_ship(SpaceShipState::new_corvette(
            "customs.4.1",
            "Corvette",
            CUSTOMS_CORVETTE_PARTY,
            [0, 0, 0],
            [0, 0, 0],
        ));
        for (id, name, at) in [
            ("hull.10", "Salt Moth", [3_000, 4_000, 12_000]),
            ("hull.11", "Tin Kettle", [-9_000, 0, 0]),
        ] {
            bubble.insert_ship(SpaceShipState::new_dray(
                id,
                name,
                CARRIER_PARTY,
                at,
                [0, 0, 0],
            ));
        }
        let posture_of = |situation: &Situation, ship: &str| {
            let mut book = ContactBook::default();
            space_mind_view(&bubble, ship, ObjId(7), &mut book, situation)
                .map(|(view, _)| view.posture)
        };
        let mut situation = Situation::open(1_000);
        for ship in ["customs.4.1", "hull.10", "hull.11"] {
            assert_eq!(
                posture_of(&situation, ship),
                Some(Posture::Return),
                "{ship}"
            );
        }
        situation
            .postures
            .insert("hull.10".to_owned(), Posture::Hold);
        situation
            .postures
            .insert("hull.11".to_owned(), Posture::Guard);
        assert_eq!(situation.posture_of("hull.10"), Posture::Hold);
        assert_eq!(posture_of(&situation, "hull.10"), Some(Posture::Hold));
        assert_eq!(posture_of(&situation, "hull.11"), Some(Posture::Guard));
        assert_eq!(
            posture_of(&situation, "customs.4.1"),
            Some(Posture::Return),
            "a hull with no row reads the default"
        );
    }

    /// Doc 44 unit MW.9's acceptance: adding a ship the observer cannot detect leaves her
    /// view the same, byte for byte. The added ship sorts ahead of every other in the bubble,
    /// which is what used to move the ids and the designator letters of the ones she holds.
    #[test]
    fn a_ship_the_observer_cannot_detect_leaves_her_view_the_same_bytes() {
        use crate::space::picket::{BELT_RAIDER_PARTY, CARRIER_PARTY, CUSTOMS_CORVETTE_PARTY};

        let field = |with_lurker: bool| {
            let mut bubble = LocalSpaceBubble::new("bubble.test.view", "domain.4", "Test", 1_000);
            bubble.insert_ship(SpaceShipState::new_corvette(
                "customs.4.1",
                "Corvette",
                CUSTOMS_CORVETTE_PARTY,
                [0, 0, 0],
                [0, 0, 0],
            ));
            bubble.insert_ship(SpaceShipState::new_dray(
                "hull.10",
                "Salt Moth",
                CARRIER_PARTY,
                [3_000, 4_000, 12_000],
                [5_000, 0, -2_000],
            ));
            bubble.insert_ship(SpaceShipState::new_dray(
                "hull.11",
                "Tin Kettle",
                CARRIER_PARTY,
                [-9_000, 0, 0],
                [0, 0, 0],
            ));
            if with_lurker {
                let mut lurker = SpaceShipState::new_corvette(
                    "a.lurker",
                    "Lurker",
                    BELT_RAIDER_PARTY,
                    [0, 150_000, 0],
                    [0, 0, 0],
                );
                lurker.emcon = EmconPosture::Ghost;
                lurker.heat_permille = 0;
                bubble.insert_ship(lurker);
            }
            bubble
        };
        let mut situation = Situation::open(1_000);
        situation.stopper = Some("customs.4.1".to_owned());
        situation.stops.insert("hull.10".to_owned(), Stop::Hailed);

        let read = |bubble: &LocalSpaceBubble, ship: &str| {
            let mut book = ContactBook::default();
            let (view, contacts) =
                space_mind_view(bubble, ship, ObjId(7), &mut book, &situation).expect("a view");
            (format!("{view:?}"), contacts, book)
        };
        let without = field(false);
        let with = field(true);
        let lurker = with.ships.get("a.lurker").expect("the lurker");
        for ship in ["customs.4.1", "hull.10", "hull.11"] {
            let observer = with.ships.get(ship).expect("the observer");
            assert!(
                !crate::space::commands::is_bubble_ship_detected(
                    observer,
                    lurker,
                    with.ic_timestamp_s
                ),
                "{ship} must not hold the lurker for this test to mean anything"
            );
            assert_eq!(read(&without, ship), read(&with, ship), "{ship}");
        }

        // And what the corvette does read of the carrier it has hailed.
        let mut book = ContactBook::default();
        let (view, contacts) =
            space_mind_view(&with, "customs.4.1", ObjId(7), &mut book, &situation).expect("a view");
        let space = view.space.as_deref().expect("the space row");
        let moth = contacts
            .iter()
            .find(|(_, ship)| ship.as_str() == "hull.10")
            .map(|(id, _)| *id)
            .expect("the corvette holds the Salt Moth");
        let row = space.contacts.get(&moth).expect("her row");
        assert_eq!(row.stop, Stop::Hailed);
        assert_eq!(row.vel_cms, [500, 0, -200]);
        // 12 km up over 5 km out: about 67 degrees.
        assert_eq!(row.elevation_deg, 67);
        assert_eq!(space.heard, Stop::Hailed);
        assert_eq!(space.stop, Stop::None);
        assert!(!view.held);
        // The carrier reads the stop on herself, and who holds it.
        let mut book = ContactBook::default();
        let (view, contacts) =
            space_mind_view(&with, "hull.10", ObjId(10), &mut book, &situation).expect("a view");
        let space = view.space.as_deref().expect("the space row");
        assert!(view.held);
        assert_eq!(space.stop, Stop::Hailed);
        assert_eq!(
            space
                .stop_by
                .and_then(|id| contacts.get(&id))
                .map(String::as_str),
            Some("customs.4.1")
        );
        assert_eq!(space.vel_cms, [500, 0, -200]);
    }

    #[test]
    fn a_contact_book_numbers_a_ship_once_and_in_the_order_it_was_first_held() {
        let mut book = ContactBook::default();
        assert!(book.is_empty());
        assert_eq!(book.number("raider.7.1"), ContactId(1));
        assert_eq!(book.number("customs.7.1"), ContactId(2));
        assert_eq!(book.number("raider.7.1"), ContactId(1));
        assert_eq!(book.held("customs.7.1"), Some(ContactId(2)));
        assert_eq!(book.held("hull.9"), None);
        let kept: ContactBook =
            serde_json::from_value(serde_json::to_value(&book).expect("encodes")).expect("decodes");
        assert_eq!(kept, book);
    }

    /// **Doc 45 unit AP.9: a hull that only answers fire is shown as hostile only what she
    /// may answer, and a ship she cannot detect still moves no byte of her view.** With a
    /// row laid for her, a contact her party holds hostile reads neutral unless it is in the
    /// row; a hull with no row reads it hostile as before. A ship her sensors do not hold,
    /// added to the bubble and to her row, leaves her view the same bytes.
    #[test]
    fn ap9_a_hull_that_answers_is_shown_hostile_only_what_she_may_answer() {
        use crate::arena::party::{Mark, Relation};
        use crate::space::picket::{BELT_RAIDER_PARTY, CARRIER_PARTY, CUSTOMS_CORVETTE_PARTY};

        let mut bubble = LocalSpaceBubble::new("bubble.test.answerable", "domain.4", "Test", 1_000);
        bubble.insert_ship(SpaceShipState::new_dray(
            "hull.10",
            "Salt Moth",
            CARRIER_PARTY,
            [0, 0, 0],
            [0, 0, 0],
        ));
        for (id, party, at) in [
            ("raider.1", BELT_RAIDER_PARTY, [8_000, 0, 0]),
            ("customs.1", CUSTOMS_CORVETTE_PARTY, [-8_000, 0, 0]),
        ] {
            let mut ship = SpaceShipState::new_corvette(id, id, party, at, [0, 0, 0]);
            ship.emcon = EmconPosture::ActiveCombat;
            bubble.insert_ship(ship);
        }
        bubble.set_party_relation(CARRIER_PARTY, BELT_RAIDER_PARTY, Some(Mark::Hostile));
        bubble.set_party_relation(CARRIER_PARTY, CUSTOMS_CORVETTE_PARTY, Some(Mark::Hostile));

        let relations = |bubble: &LocalSpaceBubble, situation: &Situation| {
            let mut book = ContactBook::default();
            let (view, map) = space_mind_view(bubble, "hull.10", ObjId(7), &mut book, situation)
                .expect("her view");
            let by_ship: BTreeMap<String, Relation> = view
                .contacts
                .iter()
                .filter_map(|row| map.get(&row.id).map(|ship| (ship.clone(), row.relation)))
                .collect();
            (by_ship, format!("{view:?}"))
        };
        let open = Situation::open(1_000);
        let (as_held, _) = relations(&bubble, &open);
        assert_eq!(as_held.get("raider.1"), Some(&Relation::Hostile));
        assert_eq!(as_held.get("customs.1"), Some(&Relation::Hostile));

        // A row with only the raider in it: customs read neutral, the raider hostile.
        let mut answering = Situation::open(1_000);
        answering.answerable.insert(
            "hull.10".to_owned(),
            std::collections::BTreeSet::from(["raider.1".to_owned()]),
        );
        let (narrowed, bytes) = relations(&bubble, &answering);
        assert_eq!(narrowed.get("raider.1"), Some(&Relation::Hostile));
        assert_eq!(narrowed.get("customs.1"), Some(&Relation::Neutral));
        // An empty row: nobody reads hostile. The row never makes a contact hostile.
        let mut nobody = Situation::open(1_000);
        nobody
            .answerable
            .insert("hull.10".to_owned(), std::collections::BTreeSet::new());
        let (none, _) = relations(&bubble, &nobody);
        assert!(none.values().all(|relation| *relation == Relation::Neutral));
        let mut widened = Situation::open(1_000);
        bubble.set_party_relation(CARRIER_PARTY, CUSTOMS_CORVETTE_PARTY, None);
        widened.answerable.insert(
            "hull.10".to_owned(),
            std::collections::BTreeSet::from(["customs.1".to_owned()]),
        );
        let (not_widened, _) = relations(&bubble, &widened);
        assert_eq!(
            not_widened.get("customs.1"),
            Some(&Relation::Neutral),
            "a ship in her row that her party does not hold hostile is not shown hostile"
        );
        bubble.set_party_relation(CARRIER_PARTY, CUSTOMS_CORVETTE_PARTY, Some(Mark::Hostile));

        // A dark ship far off that she cannot detect, sorting first, and in her row.
        let mut unseen = SpaceShipState::new_corvette(
            "a.unseen",
            "Unseen",
            BELT_RAIDER_PARTY,
            [0, 150_000, 0],
            [0, 0, 0],
        );
        unseen.emcon = EmconPosture::Ghost;
        unseen.heat_permille = 0;
        let observer = bubble.ships.get("hull.10").expect("her").clone();
        assert!(
            !crate::space::commands::is_bubble_ship_detected(
                &observer,
                &unseen,
                bubble.ic_timestamp_s
            ),
            "the rig's unseen ship is one she cannot detect"
        );
        bubble.insert_ship(unseen);
        answering
            .answerable
            .get_mut("hull.10")
            .expect("her row")
            .insert("a.unseen".to_owned());
        let (_, with_unseen) = relations(&bubble, &answering);
        assert_eq!(with_unseen, bytes, "her view is the same bytes");
    }
}

/// **Doc 49 unit CC.6: one row a mount, its maximum from the fit, its row by the fit's modes.**
#[cfg(test)]
mod cc6_tests {
    use super::*;
    use crate::arena::party::Mark;
    use crate::space::picket::{BELT_RAIDER_PARTY, CARRIER_PARTY};

    fn ids(rows: &[FrameWeapon]) -> Vec<&str> {
        rows.iter().map(|row| row.id.as_str()).collect()
    }

    /// Her rows with no lock held (doc 49 unit CC.9 reads them through her picture).
    fn rows(ship: &SpaceShipState) -> Vec<FrameWeapon> {
        weapon_rows(ship, &ship_picture(ship, None))
    }

    fn halcyon() -> SpaceShipState {
        let mut ship =
            SpaceShipState::new_corvette("halcyon", "Halcyon", CARRIER_PARTY, [0, 0, 0], [0, 0, 0]);
        super::super::custody::apply_space_unit_overlay(
            &mut ship,
            &super::super::custody::space_unit_overlay_for_id("tacsim.unit.halcyon"),
        );
        ship
    }

    #[test]
    fn cc6_every_mount_a_mind_may_fire_is_a_row() {
        let corvette =
            SpaceShipState::new_corvette("c", "Corvette", CARRIER_PARTY, [0, 0, 0], [0, 0, 0]);
        // Both lasers (the second starts in defence and may switch), the tubes, the spinal
        // gun; point defence fires on its own and has no row.
        assert_eq!(
            ids(&rows(&corvette)),
            ["laser_1", "laser_2", "torp_bow", "rail_spinal"]
        );
        let dray = SpaceShipState::new_dray("d", "Dray", CARRIER_PARTY, [0, 0, 0], [0, 0, 0]);
        assert_eq!(
            ids(&rows(&dray)),
            ["laser_dorsal", "laser_ventral"],
            "Q-49-25: both of the Dray's lasers"
        );
    }

    #[test]
    fn cc6_a_magazines_maximum_is_her_fits_and_does_not_shrink() {
        let mut bubble = LocalSpaceBubble::new("bubble.test.cc6", "domain.9", "Test", 1_000);
        bubble.insert_ship(halcyon());
        bubble.insert_ship(SpaceShipState::new_dray(
            "prey",
            "Prey",
            BELT_RAIDER_PARTY,
            [100_000, 0, 0],
            [0, 0, 0],
        ));
        bubble.set_party_relation(CARRIER_PARTY, BELT_RAIDER_PARTY, Some(Mark::Hostile));
        let max = |bubble: &LocalSpaceBubble, id: &str| {
            rows(bubble.ships.get("halcyon").expect("her ship"))
                .into_iter()
                .find(|row| row.id == id)
                .map(|row| (row.rounds, row.rounds_max))
        };
        // What her fit stows, read from content.
        let stowed = |id: &str| {
            super::super::custody::space_fit_for_id("tacsim.unit.halcyon")
                .and_then(|fit| fit.mounts.iter().find(|mount| mount.id.as_str() == id))
                .and_then(|mount| mount.magazine)
                .expect("her fit stows it")
        };
        let (tubes, rounds) = (stowed("torp_bow"), stowed("rail_spinal"));
        assert!(tubes > 8 && rounds > 24, "more than the old fixed maxima");
        assert_eq!(max(&bubble, "torp_bow"), Some((tubes, tubes)));
        assert_eq!(max(&bubble, "rail_spinal"), Some((rounds, rounds)));
        assert!(!bubble.fire_torpedo_salvo("halcyon", "prey", 2).is_empty());
        assert!(!bubble.fire_railgun("halcyon", "prey").is_empty());
        assert_eq!(
            max(&bubble, "torp_bow"),
            Some((tubes - 2, tubes)),
            "the old fixed maximum of 8 shrank it to what was left"
        );
        assert_eq!(max(&bubble, "rail_spinal"), Some((rounds - 1, rounds)));
    }

    #[test]
    fn cc6_a_laser_fitted_defence_only_is_no_row() {
        let mut dray = SpaceShipState::new_dray("d", "Dray", CARRIER_PARTY, [0, 0, 0], [0, 0, 0]);
        if let Some(laser) = dray.lasers.first_mut() {
            laser.allowed_modes = super::super::weapons::LaserModes {
                offence: false,
                defence: true,
            };
            laser.mode = super::super::weapons::LaserMode::Defense;
        }
        assert_eq!(ids(&rows(&dray)), ["laser_ventral"]);
    }
}

/// **Doc 49 unit CC.9: what her mounts and her lock read before a shot, and the mind reads the
/// same.** Space has no chance roll (Q-49-13 as corrected by doc 48b's W-49-5): the figures are
/// the kernel's own, and nothing a mind or a player reads is odds.
#[cfg(test)]
mod cc9_tests {
    use super::*;
    use crate::arena::party::Mark;
    use crate::space::picket::{BELT_RAIDER_PARTY, CARRIER_PARTY};

    fn halcyon() -> SpaceShipState {
        let mut ship =
            SpaceShipState::new_corvette("halcyon", "Halcyon", CARRIER_PARTY, [0, 0, 0], [0, 0, 0]);
        super::super::custody::apply_space_unit_overlay(
            &mut ship,
            &super::super::custody::space_unit_overlay_for_id("tacsim.unit.halcyon"),
        );
        ship
    }

    /// Halcyon and a raider at `at`, hot and radiating so her sensors hold it, locked.
    fn field(at: [i64; 3]) -> LocalSpaceBubble {
        let mut bubble = LocalSpaceBubble::new("bubble.test.cc9", "domain.9", "Test", 1_000);
        let mut ship = halcyon();
        ship.locked_target_id = Some("raider".to_owned());
        bubble.insert_ship(ship);
        let mut raider =
            SpaceShipState::new_corvette("raider", "Belt Raider", BELT_RAIDER_PARTY, at, [0, 0, 0]);
        raider.emcon = EmconPosture::ActiveCombat;
        raider.heat_permille = 600;
        bubble.insert_ship(raider);
        bubble.set_party_relation(CARRIER_PARTY, BELT_RAIDER_PARTY, Some(Mark::Hostile));
        bubble
    }

    fn held(bubble: &LocalSpaceBubble) -> bool {
        super::super::commands::is_bubble_ship_detected(
            &bubble.ships["halcyon"],
            &bubble.ships["raider"],
            bubble.ic_timestamp_s,
        )
    }

    fn mount<'a>(picture: &'a FirePicture, id: &str) -> &'a MountFacts {
        picture
            .mounts
            .iter()
            .find(|mount| mount.id == id)
            .unwrap_or_else(|| panic!("she carries {id}"))
    }

    #[test]
    fn cc9_every_mount_reads_its_rounds_mode_and_ready_and_the_lock_its_range_and_band() {
        let bubble = field([-60_000, 0, 0]);
        assert!(held(&bubble), "the rig's raider is one her sensors hold");
        let picture = fire_picture(&bubble, "halcyon").expect("her picture");
        let ids: Vec<&str> = picture
            .mounts
            .iter()
            .map(|mount| mount.id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "laser_dorsal",
                "laser_ventral",
                "torp_bow",
                "rail_spinal",
                "pdc_port",
                "pdc_stbd",
                "pdc_dorsal",
                "pdc_ventral"
            ],
            "Halcyon's full fit: every mount, point defense among them"
        );
        let ship = &bubble.ships["halcyon"];
        let dorsal = mount(&picture, "laser_dorsal");
        assert_eq!(dorsal.mode, Some(LaserMode::Offense));
        assert_eq!((dorsal.rounds, dorsal.rounds_max), (None, None));
        assert_eq!(dorsal.state, MountState::Ready);
        assert_eq!(
            dorsal.at_lock,
            Some(AtLock::Damage(ship.lasers[0].damage_at_range(60_000))),
            "the kernel's own damage at range"
        );
        let spinal = mount(&picture, "rail_spinal");
        assert_eq!(
            (spinal.rounds, spinal.rounds_max),
            (Some(32), Some(32)),
            "what her fit stows"
        );
        let target = &bubble.ships["raider"];
        let (_, flight) = compute_railgun_lead(
            ship.pos_m,
            ship.vel_mmps,
            target.pos_m,
            target.vel_mmps,
            ship.railguns[0].muzzle_vel_mps,
        );
        assert!(flight > 0);
        assert_eq!(spinal.at_lock, Some(AtLock::FlightMs(flight)));
        assert_eq!(mount(&picture, "torp_bow").rounds, Some(16));
        assert_eq!(mount(&picture, "torp_bow").at_lock, None);
        assert_eq!(mount(&picture, "pdc_port").rounds, Some(4_000));
        let lock = picture.lock.as_ref().expect("the lock is held");
        assert_eq!((lock.range_m, lock.facet), (60_000, FacetBand::Front));
        assert_eq!(picture.lock_id.as_deref(), Some("raider"));
    }

    /// The five bands: the band of her hull a shot from here strikes, top among them, as the
    /// kernel's own facet rule picks it, on the picture, on the mind's space row and through
    /// `Contact.aspect`.
    #[test]
    fn cc9_the_lock_reads_her_band_of_five_and_a_mind_reads_the_same() {
        use crate::minds::fixture::{context_on, eval_in, shown};
        for (at, band) in [
            ([-60_000, 0, 0], FacetBand::Front),
            ([60_000, 0, 0], FacetBand::Rear),
            ([0, -60_000, 0], FacetBand::Right),
            ([0, 60_000, 0], FacetBand::Left),
            ([0, 0, -60_000], FacetBand::Top),
        ] {
            let bubble = field(at);
            assert!(held(&bubble), "{at:?}");
            let picture = fire_picture(&bubble, "halcyon").expect("her picture");
            assert_eq!(picture.lock.as_ref().map(|lock| lock.facet), Some(band));
            let mut book = ContactBook::default();
            let (view, _) = space_mind_view(
                &bubble,
                "halcyon",
                ObjId(7),
                &mut book,
                &Situation::open(1_000),
            )
            .expect("her view");
            let row = &view.space.as_deref().expect("space").contacts[&view.contacts[0].id];
            assert_eq!(row.facet, band);
            assert!(view.contacts[0].odds.is_empty(), "no odds in space");
            let read = eval_in(context_on(view), "v.contacts[0].aspect").expect("reads");
            assert_eq!(shown(&read), format!("{:?}", band.id_str()), "{at:?}");
        }
    }

    /// What holds a mount back is the first that holds, by the kernel's own rules: the cycle,
    /// the magazine, the heat ceiling, the reach, a defence mode, a rotary that is down.
    #[test]
    fn cc9_a_mount_reads_what_holds_it_back() {
        let mut bubble = field([-60_000, 0, 0]);
        {
            let ship = bubble.ships.get_mut("halcyon").expect("her");
            ship.lasers[0].cooldown_remaining_ms = 2_350;
            ship.lasers[1].mode = LaserMode::Defense;
            ship.railguns[0].rounds_remaining = 0;
            ship.railguns[0].fired_since_full = 32;
            ship.torpedo_tubes[0].cooldown_remaining_ms = 4_000;
            ship.pdcs[0].operational = false;
            ship.pdcs[1].ammo_remaining = 0;
        }
        let picture = fire_picture(&bubble, "halcyon").expect("her picture");
        let dorsal = mount(&picture, "laser_dorsal");
        assert_eq!(
            (dorsal.state, dorsal.ready_in_ms),
            (MountState::Cycling, 2_350)
        );
        let ventral = mount(&picture, "laser_ventral");
        assert_eq!(
            (ventral.mode, ventral.state),
            (Some(LaserMode::Defense), MountState::Ready),
            "a laser in defense is ready to stop a torpedo"
        );
        assert!(!ventral.cycled(), "and not to fire at her lock");
        // Doc 48e kernel G5: in defence it reaches its defence range, not its offence reach.
        let defence_reach = bubble.ships["halcyon"].lasers[1].defense_range_m;
        assert_eq!(ventral.reach_m, Some(defence_reach));
        assert_ne!(defence_reach, bubble.ships["halcyon"].lasers[1].max_range_m);
        let spinal = mount(&picture, "rail_spinal");
        assert_eq!(
            (spinal.state, spinal.rounds, spinal.rounds_max),
            (MountState::Empty, Some(0), Some(32))
        );
        assert_eq!(mount(&picture, "torp_bow").state, MountState::Cycling);
        assert_eq!(mount(&picture, "pdc_port").state, MountState::Down);
        assert_eq!(mount(&picture, "pdc_stbd").state, MountState::Empty);
        // A hull disabled to a derelict fires nothing: every mount is down.
        let mut wreck = bubble.clone();
        wreck.ships.get_mut("halcyon").expect("her").status = HullStatus::DisabledDerelict {
            epoch_open: true,
            life_support_remaining_s: 60,
        };
        let picture = fire_picture(&wreck, "halcyon").expect("her picture");
        assert!(
            picture
                .mounts
                .iter()
                .all(|mount| mount.state == MountState::Down)
        );

        // One more pulse would cross her heat ceiling: the kernel does not take it.
        {
            let ship = bubble.ships.get_mut("halcyon").expect("her");
            ship.lasers[0].cooldown_remaining_ms = 0;
            ship.lasers[1].cooldown_remaining_ms = 1_000;
            ship.heat_permille = HEAT_CEILING_PERMILLE - ship.lasers[0].heat_cost_permille + 1;
        }
        let picture = fire_picture(&bubble, "halcyon").expect("her picture");
        assert_eq!(mount(&picture, "laser_dorsal").state, MountState::TooHot);
        // Doc 48e kernel G5: a laser in defence as hot reads too hot as well; the kernel
        // refuses its intercept.
        {
            let ship = bubble.ships.get_mut("halcyon").expect("her");
            ship.lasers[1].cooldown_remaining_ms = 0;
            ship.heat_permille = HEAT_CEILING_PERMILLE - ship.lasers[1].heat_cost_permille + 1;
        }
        let picture = fire_picture(&bubble, "halcyon").expect("her picture");
        assert_eq!(mount(&picture, "laser_ventral").state, MountState::TooHot);
        {
            let ship = bubble.ships.get_mut("halcyon").expect("her");
            ship.lasers[1].cooldown_remaining_ms = 1_000;
            ship.heat_permille = HEAT_CEILING_PERMILLE - ship.lasers[0].heat_cost_permille + 1;
        }
        assert!(
            bubble.fire_offensive_lasers("halcyon", "raider").is_empty(),
            "the kernel refuses the same shot"
        );

        // Past the laser's reach and inside the spinal gun's (doc 49 unit CC.8 made the
        // laser's the shorter): the laser reads past reach, the spinal gun fires.
        let laser_reach = bubble.ships["halcyon"].lasers[0].max_range_m;
        let spinal_reach = bubble.ships["halcyon"].railguns[0].max_range_m;
        assert!(laser_reach < spinal_reach);
        let between = field([-(laser_reach + 1_000), 0, 0]);
        if held(&between) {
            let picture = fire_picture(&between, "halcyon").expect("her picture");
            assert_eq!(mount(&picture, "laser_dorsal").state, MountState::PastReach);
            assert_eq!(mount(&picture, "laser_dorsal").at_lock, None);
            assert_eq!(mount(&picture, "rail_spinal").state, MountState::Ready);
            assert!(mount(&picture, "rail_spinal").at_lock.is_some());
        }

        // Past a mount's reach: the lock as far off as her sensors still hold her.
        let reach = laser_reach.max(spinal_reach);
        let far = field([-(reach + 1_000), 0, 0]);
        if held(&far) {
            let picture = fire_picture(&far, "halcyon").expect("her picture");
            assert_eq!(mount(&picture, "laser_dorsal").state, MountState::PastReach);
            assert_eq!(mount(&picture, "laser_dorsal").at_lock, None);
            assert_eq!(mount(&picture, "rail_spinal").state, MountState::PastReach);
            assert_eq!(mount(&picture, "rail_spinal").at_lock, None);
        } else {
            // The lock lost: no lock figures at all, and the mounts read their own state.
            let picture = fire_picture(&far, "halcyon").expect("her picture");
            assert_eq!(picture.lock, None);
            assert_eq!(picture.lock_id.as_deref(), Some("raider"));
            assert!(picture.mounts.iter().all(|mount| mount.at_lock.is_none()));
        }
    }

    /// A mind reads the same: its rows of `v.weapons` carry no odds, the true reach, heat and
    /// readiness, and `space_mount(v, id)` reads every mount, point defence among them.
    #[test]
    fn cc9_a_space_mind_reads_the_kernels_figures_for_every_mount() {
        use crate::minds::fixture::{context_on, eval_in, shown};
        let mut bubble = field([-60_000, 0, 0]);
        bubble.ships.get_mut("halcyon").expect("her").lasers[1].mode = LaserMode::Defense;
        let mut book = ContactBook::default();
        let (view, _) = space_mind_view(
            &bubble,
            "halcyon",
            ObjId(7),
            &mut book,
            &Situation::open(1_000),
        )
        .expect("her view");
        let ship = &bubble.ships["halcyon"];
        for row in &view.weapons {
            assert_eq!(row.p_permille, None, "{}: no odds in space", row.id);
        }
        let row = |id: &str| {
            view.weapons
                .iter()
                .find(|row| row.id == id)
                .unwrap_or_else(|| panic!("a row for {id}"))
        };
        assert_eq!(
            row("laser_dorsal").bands_cm,
            [u32::try_from(ship.lasers[0].max_range_m * 100).expect("fits")]
        );
        assert_eq!(
            row("laser_dorsal").heat_per_shot,
            u32::from(ship.lasers[0].heat_cost_permille)
        );
        assert!(row("laser_dorsal").ready);
        assert!(!row("laser_ventral").ready, "in defense");
        assert_eq!(
            row("laser_ventral").ready_in_ms,
            0,
            "its cycle has finished"
        );
        assert_eq!(
            row("rail_spinal").bands_cm,
            [u32::try_from(ship.railguns[0].max_range_m * 100).expect("fits")]
        );
        assert!(
            row("torp_bow").bands_cm.is_empty(),
            "the tubes declare no reach"
        );
        assert!(
            view.weapons.iter().all(|row| !row.id.starts_with("pdc")),
            "point defense fires on its own"
        );

        let read = |expr: &str| shown(&eval_in(context_on(view.clone()), expr).expect(expr));
        let damage = ship.lasers[0].damage_at_range(60_000);
        assert_eq!(read("space_mount(v, \"laser_dorsal\").family"), "\"laser\"");
        assert_eq!(read("space_mount(v, \"laser_dorsal\").mode"), "\"offence\"");
        assert_eq!(
            read("space_mount(v, \"laser_ventral\").mode"),
            "\"defence\""
        );
        assert_eq!(read("space_mount(v, \"laser_dorsal\").state"), "\"ready\"");
        assert_eq!(
            read("space_mount(v, \"laser_dorsal\").lock_damage"),
            damage.to_string()
        );
        assert_eq!(read("space_mount(v, \"laser_dorsal\").rounds"), "()");
        assert_eq!(read("space_mount(v, \"rail_spinal\").rounds_max"), "32");
        assert_ne!(read("space_mount(v, \"rail_spinal\").lock_flight_ms"), "()");
        assert_eq!(read("space_mount(v, \"torp_bow\").reach_m"), "()");
        assert_eq!(
            read("space_mount(v, \"pdc_port\").family"),
            "\"point_defence\""
        );
        assert_eq!(read("space_mount(v, \"pdc_port\").rounds"), "4000");
        assert_eq!(read("space_mount(v, \"no_such_mount\")"), "()");
        // On the ground there is no space mount.
        assert_eq!(
            shown(
                &eval_in(
                    context_on(crate::minds::fixture::view()),
                    "space_mount(v, \"cannon\")"
                )
                .expect("reads")
            ),
            "()"
        );
    }

    /// **The 120-column floor at the widest fill on Halcyon's full fit**: every row
    /// `+tac/status` prints, two spaces in, with every field at the widest it can be: a cycle
    /// of an hour left, a full magazine at the type's bound, a pulse at the damage bound and a
    /// spinal round at its longest flight.
    #[test]
    fn cc9_every_status_row_fits_120_columns_at_the_widest_fill() {
        use crate::definitions::space_mount::{
            MAX_PD_MAGAZINE, MAX_SPACE_CYCLE_MS, MAX_SPACE_DAMAGE,
        };
        // A pulse lands its whole damage inside a kilometre; a slow round flies longest far off.
        let mut lines = Vec::new();
        for at in [[-900, 0, 0], [-60_000, 0, 0]] {
            let mut bubble = field(at);
            assert!(held(&bubble), "{at:?}");
            let ship = bubble.ships.get_mut("halcyon").expect("her");
            for laser in &mut ship.lasers {
                laser.cooldown_remaining_ms = MAX_SPACE_CYCLE_MS;
                laser.base_damage = MAX_SPACE_DAMAGE;
            }
            for tube in &mut ship.torpedo_tubes {
                tube.cooldown_remaining_ms = MAX_SPACE_CYCLE_MS;
                tube.magazine_remaining = u16::MAX;
            }
            for gun in &mut ship.railguns {
                gun.cooldown_remaining_ms = MAX_SPACE_CYCLE_MS;
                gun.rounds_remaining = u16::MAX;
                gun.muzzle_vel_mps = 1_000;
            }
            for pd in &mut ship.pdcs {
                pd.cooldown_remaining_ms = MAX_SPACE_CYCLE_MS;
                pd.ammo_remaining = MAX_PD_MAGAZINE;
            }
            let picture = fire_picture(&bubble, "halcyon").expect("her picture");
            let rows = super::super::commands::fire_status_lines(&picture);
            assert_eq!(rows.len(), 10, "the head, eight mounts, the lock");
            lines.extend(rows);
        }
        for line in &lines {
            assert!(
                line.chars().count() <= 120,
                "{} columns: {line}",
                line.chars().count()
            );
        }
        for needle in [
            "in 3600.0 s",
            "100000 a pulse",
            "65535/65535",
            "1000000 left",
            "flies 60.0 s",
        ] {
            assert!(
                lines.iter().any(|line| line.contains(needle)),
                "{needle}: {lines:#?}"
            );
        }
    }
}
