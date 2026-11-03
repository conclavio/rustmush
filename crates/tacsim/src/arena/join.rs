//! **A unit joining a running field at its pad** (doc 36 cut GC.1b; Q-36-4, doc 32 F-6 and
//! Q-CF-16).
//!
//! The kernel had no way to add a unit to a running arena: [`Arena::new_with_minds`] was the only
//! builder of a unit row. A carrier that settles on a promoted site after admission is on the
//! ground, and Q-CF-16 puts everyone on the ground in the field, so the IC field host sends
//! [`crate::arena_task::ArenaMsg::Join`] with the hull's check-out row; at the next pulse's head,
//! before the step (a mark's and a withdrawal's boundary), [`Arena::join`] admits it through the
//! one per-unit path the check-out takes (`Arena::admit_unit`: its seal or cards, its sensors and
//! guns, its band cache), and the answer rides [`crate::arena_task::ArenaOut::Joined`].
//!
//! **What it joins as.** A unit of its own party: the party the host names, whose book is opened
//! now if the field has none yet, with the host's standing friendships and marks that name it
//! (Q-CF-5, Q-CF-44). Its letter is drawn from those its party has not given, by the check-out's
//! own keyed mix of the seed, the party and the unit id (Q-CF-11), so no other unit's name moves.
//! It stands at its pose, or — when another unit holds that cell — on the nearest free cell it may
//! stand on within `spread` cells (nearest ring first, row-major within a ring;
//! `field::carrier::pad_poses`' rule, read against the arena's own occupancy). Its first sensor
//! pass and, for a mind's hull, its first think fall at its next slot of their stagger grids
//! (`Arena::grid_at`). One `mind_sealed` line journals it, as every unit's at check-out.
//!
//! **What it never joins.** A squad (squads are the OOC lobby's command, dealt at check-out); a
//! scripted mind (an IC field carries none until doc 23 S.4b, Q-CF-43, and the task holds no
//! source); an OOC match: only an arena its host ends ([`EndCondition::Host`], the IC field's)
//! takes a unit in, under a host that admits every class, and the task refuses the message
//! outright without field terms, so no practice hash can ever see one.
//!
//! **All or nothing.** The join is built on a copy of the arena and swapped in whole, so a refusal
//! leaves the arena exactly as it was.

use mush_core::ObjId;

use crate::arena::mind::SealEngine;
use crate::arena::party::{EndCondition, FieldHost, LETTERS};
use crate::arena::state::{
    Arena, ArenaError, CostRow, CustodyUnit, Designator, DueKind, PartyId, Pilot,
};

/// The cells `ring` cells from `centre` (Chebyshev), row-major; `ring 0` is the centre.
#[must_use]
pub fn ring_cells(centre: (u16, u16), ring: u16) -> Vec<(u16, u16)> {
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

/// A refusal of [`Arena::join`] that no check-out meets.
fn refused(unit: ObjId, reason: &str) -> ArenaError {
    ArenaError::Mind {
        unit,
        reason: format!("it cannot join the field: {reason}"),
    }
}

impl Arena {
    /// **Give a party arriving mid-field its home** (doc 36 cut GC.11; Q-36-55): a raider group
    /// that joins a running field (its carrier and hulls landed after admission) goes home up its
    /// own carrier's ramp like a party admitted at the opening, so the ramp's zone is laid on the
    /// field's ground ([`crate::arena::state::ArenaDefinition::homes`]) at the next pulse's head,
    /// before the joins. A party that has a home keeps it, and a zone off the ground is refused.
    /// Answers whether the ground gained it. Only an IC field's arena takes one, as with a join.
    pub fn add_home(&mut self, zone: crate::arena::state::EntryZone) -> bool {
        let terrain = &self.definition.terrain;
        if self.end != EndCondition::Host
            || self
                .definition
                .homes
                .iter()
                .any(|home| home.team == zone.team)
            || terrain.get(zone.x0, zone.y0).is_none()
            || terrain.get(zone.x1, zone.y1).is_none()
        {
            return false;
        }
        std::sync::Arc::make_mut(&mut self.definition)
            .homes
            .push(zone);
        true
    }

    /// **Admit `unit` into the running field** (this module's head), seated by `pilot` when one
    /// sits in it, its party's book opened from `host` when the field has none. Answers the name
    /// it joined under.
    ///
    /// # Errors
    ///
    /// [`ArenaError`] naming the unit — a check-out's refusals, a unit already here, a host that
    /// is not the IC field's, a scripted mind, or no free cell within `spread` of its pose — and
    /// the arena is unchanged.
    pub fn join(
        &mut self,
        unit: CustodyUnit,
        pilot: Option<Pilot>,
        host: &FieldHost,
        spread: u16,
    ) -> Result<Designator, ArenaError> {
        let mut next = self.clone();
        let name = next.join_in_place(unit, pilot, host, spread)?;
        *self = next;
        Ok(name)
    }

    fn join_in_place(
        &mut self,
        mut unit: CustodyUnit,
        pilot: Option<Pilot>,
        host: &FieldHost,
        spread: u16,
    ) -> Result<Designator, ArenaError> {
        let id = unit.id;
        if self.unit(id).is_some() {
            return Err(ArenaError::DuplicateUnit { unit: id });
        }
        // The arena's own end says whose it is: only the IC host ends its field (Q-CF-20), and an
        // OOC match ends by elimination, whatever host a caller hands it.
        if self.end != EndCondition::Host || !host.every_class || host.all_hostile {
            return Err(refused(id, "only an IC field takes a unit in once it runs"));
        }
        if unit.seal.engine == SealEngine::Scripted {
            return Err(refused(id, "a scripted mind is never taken into a field"));
        }
        if let Some(pilot) = &pilot
            && pilot.unit != id
        {
            return Err(ArenaError::UnknownPilotUnit {
                unit: pilot.unit,
                pilot: pilot.pilot,
                station: pilot.station.clone(),
            });
        }
        unit.pose = self.free_pose(&unit, spread)?;
        let class = unit.seal.class;
        let mut admitted = crate::arena::conditions::ConditionTable::default();
        let (mut state, line) = Self::admit_unit(
            unit,
            &self.definition.terrain,
            &self.sensors_cfg,
            &self.classes,
            self.odds_floor_permille,
            host.weapons_hold,
            &mut self.cards,
            &mut self.profiles,
            &mut admitted,
        )?;
        let party = state.team;
        state.callsign = self.joining_name(party, id)?;
        let name = state.callsign;
        let controller = state.controller;
        let at = self
            .units
            .binary_search_by_key(&id, |state| state.id)
            .err()
            .ok_or(ArenaError::DuplicateUnit { unit: id })?;
        self.units.insert(at, state);
        // Doc 36a lane 3 (P1-9): the rules `admit_unit` installed went into a scratch table; a
        // mind hull's native profile rows go into the arena's own (a piloted hull keeps none, by
        // design: `crate::arena::conditions`).
        if controller == super::state::Controller::Mind
            && let Some(runtime) = self.profiles.get(&id)
        {
            runtime
                .program
                .install(&mut self.conditions, id, self.classes.row(class).slots)
                .map_err(|error| {
                    refused(
                        id,
                        &format!("its profile rules were not installed: {error}"),
                    )
                })?;
        }
        super::state::check_substep(&self.definition.terrain, &self.units, &self.min_cost)?;
        if !self.relations.contains(party) {
            self.relations.admit(party);
            for (a, b) in host
                .friends
                .iter()
                .filter(|(a, b)| *a == party || *b == party)
            {
                self.relations.befriend(*a, *b);
            }
            for (by, of, mark) in host
                .marks
                .iter()
                .filter(|(by, of, _)| *by == party || *of == party)
            {
                self.relations.mark(*by, *of, Some(*mark));
            }
        }
        if let Some(pilot) = pilot {
            super::state::raise_seat(&mut self.epochs, &pilot);
            self.pilots.push(pilot);
        }
        self.chassis = super::state::ChassisWords::table(&self.units);
        let terrain = std::sync::Arc::clone(&self.definition);
        self.index.rebuild(&terrain.terrain, &self.units);
        self.alert_range_cm =
            super::state::alert_range_cm(&self.arena_cfg, &terrain.terrain, &self.units);
        self.jam_radius_cm = super::state::jam_radius_cm(&self.units);
        self.constrain_profile_hardware(id);
        let cadence = i64::from(self.sensors_cfg.pass_ms.max(1));
        let first_pass = self.grid_at(id, cadence);
        self.due.schedule(
            first_pass,
            DueKind::SensorPass {
                unit: id,
                pass_seq: 1,
            },
        );
        if self
            .unit(id)
            .is_some_and(|state| state.controller == super::state::Controller::Mind)
        {
            self.adopt_mind(id, None);
        }
        let hash = crate::arena::hash::state_hash(self);
        let _ = self.ledger.append(self.now_ms, line, Vec::new(), hash);
        super::snapshot_capacity::admit(self).map_err(|error| ArenaError::SnapshotCapacity {
            reason: error.to_string(),
        })?;
        Ok(name)
    }

    /// **Where a joining unit stands**: its own pose when its cell is free, else the centre of
    /// the nearest free cell it may stand on within `spread` (this module's head). A pose off the
    /// window or a bad heading is the check-out's to refuse, so it is answered unchanged.
    fn free_pose(
        &self,
        unit: &CustodyUnit,
        spread: u16,
    ) -> Result<mush_sim::data::Pose, ArenaError> {
        let terrain = &self.definition.terrain;
        let Some(cell) = terrain.cell_of(&unit.pose) else {
            return Ok(unit.pose.clone());
        };
        let held: std::collections::BTreeSet<(u16, u16)> = self
            .units
            .iter()
            .filter_map(|state| terrain.cell_of(&state.pose))
            .collect();
        if !held.contains(&cell) {
            return Ok(unit.pose.clone());
        }
        let row = CostRow::read(&unit.unit);
        let free = (1..=spread)
            .flat_map(|ring| ring_cells(cell, ring))
            .find(|at| !held.contains(at) && terrain.passable_for(row, at.0, at.1))
            .ok_or_else(|| refused(unit.id, "no free ground is near its pad"))?;
        let side = i64::from(terrain.cell_cm());
        let mut pose = unit.pose.clone();
        pose.x_cm = i64::from(free.0) * side + side / 2;
        pose.y_cm = i64::from(free.1) * side + side / 2;
        Ok(pose)
    }

    /// **The name a joining unit takes in `party`** (Q-CF-11): a letter of [`LETTERS`] its party
    /// has not given, picked by the check-out's keyed mix of the seed, the party and the unit id;
    /// past the letters, a pair the same way.
    fn joining_name(&self, party: PartyId, id: ObjId) -> Result<Designator, ArenaError> {
        let mut letters: Vec<u8> = LETTERS.to_vec();
        let mut pairs: Vec<u16> = (0..23 * 23).collect();
        for state in self.units.iter().filter(|state| state.team == party) {
            match state.callsign {
                Designator::Unit { letter, .. } => letters.retain(|l| char::from(*l) != letter),
                Designator::Wide { index, .. } => pairs.retain(|p| *p != index),
                Designator::Provisional(_) => {}
            }
        }
        let key = crate::arena::rng::splitmix64(
            self.seed ^ (u64::from(party.code()) << 40) ^ u64::from(id.0),
        );
        let pick = |len: usize| usize::try_from(key % len.max(1) as u64).unwrap_or(0);
        if !letters.is_empty() {
            let at = pick(letters.len()).min(letters.len() - 1);
            return Ok(Designator::Unit {
                party,
                letter: char::from(letters[at]),
            });
        }
        if pairs.is_empty() {
            return Err(ArenaError::PartyFull {
                party: party.as_str(),
            });
        }
        let at = pick(pairs.len()).min(pairs.len() - 1);
        Ok(Designator::Wide {
            party,
            index: pairs[at],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::party::{Mark, Relation};
    use crate::arena::state::Power;
    use crate::arena::test_support::{custody, line_tank, pose};
    use crate::config::{ArenaConfig, MatchConfig, SensorsConfig};

    const MINER: ObjId = ObjId(4);
    const RAIDER: ObjId = ObjId(9);
    const ARRIVAL: ObjId = ObjId(6);

    fn party(n: usize) -> PartyId {
        PartyId::nth(n).expect("a letter")
    }

    /// The IC host: every class, weapons hold, the host's end; the raider marks the miner.
    fn ic_host() -> FieldHost {
        FieldHost {
            all_hostile: false,
            marks: vec![(party(1), party(0), Mark::Hostile)],
            friends: Vec::new(),
            end: EndCondition::Host,
            every_class: true,
            weapons_hold: true,
        }
    }

    /// A running two-party IC field: the miner `A` and the raider `B`.
    fn field() -> Arena {
        let mut out = custody(
            vec![
                line_tank(MINER, party(0), pose(1_500, 1_500)),
                line_tank(RAIDER, party(1), pose(5_500, 5_500)),
            ],
            Vec::new(),
        );
        out.host = ic_host();
        let mut arena = Arena::new(
            out,
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .expect("the field");
        for state in &mut arena.units {
            state.power = Power::Online;
        }
        for wall in (100..=1_000).step_by(100) {
            arena.step_settled(wall, &[], &[]);
        }
        arena
    }

    /// The arrival's own party of one, `C`, friends with nobody but marked by the raider.
    fn arrival_host() -> FieldHost {
        FieldHost {
            marks: vec![(party(1), party(2), Mark::Hostile)],
            friends: vec![(party(0), party(2))],
            ..ic_host()
        }
    }

    #[test]
    fn gc1b_a_unit_joins_a_running_field_as_a_party_of_its_own() {
        let mut arena = field();
        let before = arena.ledger.digest();
        let name = arena
            .join(
                line_tank(ARRIVAL, party(2), pose(3_500, 3_500)),
                None,
                &arrival_host(),
                8,
            )
            .expect("it joins");
        let state = arena.unit(ARRIVAL).expect("in the field");
        assert_eq!(state.callsign, name);
        assert!(matches!(name, Designator::Unit { party: p, .. } if p == party(2)));
        assert!(state.weapons_hold, "an IC seat opens at hold (Q-CF-41)");
        assert_eq!(
            arena.units.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![MINER, ARRIVAL, RAIDER],
            "in unit-id order"
        );
        assert_eq!(arena.chassis.len(), arena.units.len());
        assert_eq!(arena.relation(party(2), party(0)), Relation::Friend);
        assert_eq!(arena.relation(party(1), party(2)), Relation::Hostile);
        assert_eq!(
            arena.relation(party(1), party(0)),
            Relation::Hostile,
            "the books it found are kept"
        );
        assert!(
            arena.due.iter().any(|event| matches!(
                event.kind,
                DueKind::SensorPass { unit, pass_seq: 1 } if unit == ARRIVAL
            ) && event.t_ms >= arena.now_ms),
            "its first pass is ahead"
        );
        assert_ne!(arena.ledger.digest(), before, "its mind_sealed line");
        for wall in (1_100..=4_000).step_by(100) {
            arena.step_settled(wall, &[], &[]);
        }
        assert!(arena.unit(ARRIVAL).is_some() && arena.ended.is_none());
    }

    #[test]
    fn gc1b_a_joiner_in_a_held_cell_stands_beside_it_or_is_refused_whole() {
        let mut arena = field();
        let held = arena.unit(MINER).expect("the miner").pose.clone();
        let hash = crate::arena::hash::state_hash(&arena);
        let refused = arena.join(
            line_tank(ARRIVAL, party(2), held.clone()),
            None,
            &arrival_host(),
            0,
        );
        assert!(refused.is_err(), "no spread, no free cell");
        assert_eq!(crate::arena::hash::state_hash(&arena), hash, "unchanged");
        assert!(arena.unit(ARRIVAL).is_none());
        arena
            .join(line_tank(ARRIVAL, party(2), held), None, &arrival_host(), 8)
            .expect("it stands beside the miner");
        let terrain = &arena.definition.terrain;
        let miner = terrain.cell_of(&arena.unit(MINER).unwrap().pose).unwrap();
        let joined = terrain.cell_of(&arena.unit(ARRIVAL).unwrap().pose).unwrap();
        assert_eq!(
            miner.0.abs_diff(joined.0).max(miner.1.abs_diff(joined.1)),
            1,
            "the nearest ring"
        );
    }

    #[test]
    fn gc1b_a_joiner_of_a_party_on_the_field_takes_a_letter_it_has_not_given() {
        let mut arena = field();
        let miner = arena.unit(MINER).unwrap().callsign;
        let name = arena
            .join(
                line_tank(ARRIVAL, party(0), pose(3_500, 3_500)),
                None,
                &ic_host(),
                8,
            )
            .expect("it joins party A");
        assert_ne!(name, miner);
        assert_eq!(
            arena.unit(MINER).unwrap().callsign,
            miner,
            "nobody else's name moves"
        );
        assert_eq!(
            arena.join(
                line_tank(ARRIVAL, party(0), pose(4_500, 4_500)),
                None,
                &ic_host(),
                8
            ),
            Err(ArenaError::DuplicateUnit { unit: ARRIVAL })
        );
    }

    #[test]
    fn gc1b_an_ooc_match_never_joins() {
        let mut arena = crate::arena::test_support::arena_of(vec![
            line_tank(MINER, PartyId::RED, pose(1_500, 1_500)),
            line_tank(RAIDER, PartyId::BLUE, pose(5_500, 5_500)),
        ]);
        let hash = crate::arena::hash::state_hash(&arena);
        assert!(
            arena
                .join(
                    line_tank(ARRIVAL, PartyId::RED, pose(3_500, 3_500)),
                    None,
                    &FieldHost::all_hostile(),
                    8,
                )
                .is_err()
        );
        assert_eq!(crate::arena::hash::state_hash(&arena), hash);
    }
}
