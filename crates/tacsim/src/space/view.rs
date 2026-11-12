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

use super::bubble::{EmconPosture, HullStatus, LocalSpaceBubble, SpaceShipState, SubsystemCrit};
use super::picket::{HEAVE_TO_REL_VEL_MMPS, relative_speed_3d_mmps};
use super::weapons::isqrt_i128;
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

/// The mounts a mind may fire, as the rows a script reads them by: her first laser, tube and
/// railgun.
fn weapon_rows(observer: &SpaceShipState) -> Vec<FrameWeapon> {
    let row = |id: &str, ready: bool, ready_in_ms: u32, rounds: u32, rounds_max: u32| FrameWeapon {
        id: SmolStr::new(id),
        ready,
        ready_in_ms: i64::from(ready_in_ms),
        rounds,
        rounds_max,
        range_band: None,
        p_permille: Some(850),
        bands_cm: vec![25_000_000],
        heat_per_shot: 0,
        armed: None,
    };
    let mut weapons = Vec::new();
    if let Some(laser) = observer.lasers.first() {
        weapons.push(row(
            &laser.id,
            laser.cooldown_remaining_ms == 0,
            laser.cooldown_remaining_ms,
            100,
            100,
        ));
    }
    if let Some(tube) = observer.torpedo_tubes.first() {
        weapons.push(row(
            &tube.id,
            tube.cooldown_remaining_ms == 0 && tube.magazine_remaining > 0,
            tube.cooldown_remaining_ms,
            u32::from(tube.magazine_remaining),
            u32::from(tube.magazine_remaining.max(8)),
        ));
    }
    if let Some(rail) = observer.railguns.first() {
        weapons.push(row(
            &rail.id,
            rail.cooldown_remaining_ms == 0 && rail.rounds_remaining > 0,
            rail.cooldown_remaining_ms,
            u32::from(rail.rounds_remaining),
            u32::from(rail.rounds_remaining.max(24)),
        ));
    }
    weapons
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
        rows.insert(
            cid,
            SpaceContactView {
                elevation_deg: libm::round(elevation_mdeg as f64 / 1_000.0) as i32,
                vel_cms: cms(other.vel_mmps),
                stop: situation.stop_of(&other.id),
                emcon: other.emcon,
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
            aspect: Aspect::Front,
            odds: Vec::new(),
            reads_dead: Some(other.status != HullStatus::Operational),
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
        weapons: weapon_rows(observer),
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
}
