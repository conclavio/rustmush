//! **Squads** (16b cut N.2, N3; doc 16 §6 "Deliberation": *requested per squad or
//! commander rather than per unit*).
//!
//! Doc 16 §6 asks for one decision per squad and not one per unit, and that is the
//! whole reason this type exists: a field of three hundred reflex units asking for
//! an order each would be three hundred provider calls a minute, and a field of
//! three hundred units asking *nothing* would be a field on doctrine for ever. A
//! squad is the unit of deliberation — one commander, one contact set, one order.
//!
//! # The grouping
//!
//! A side's reflex units, in unit-id order, cut into [`SQUAD_SIZE`]-unit squads —
//! **pod pairs**, because the arena's pods are claimed in pairs and two tanks that
//! start beside each other are two tanks that fight together. A side with fewer
//! than [`MIN_UNITS_TO_SPLIT`] reflex units gets **one** squad instead, because
//! splitting three tanks into two squads doubles the cost of commanding them and
//! buys nothing.
//!
//! The **commander** is the lowest unit id in the squad. Not a role, not a
//! capability, not a station: it is the unit whose contact set the trigger reads
//! and whose opaque contact ids an order's target hint is written in. It is the
//! lowest id for the same reason every other tie in this engine breaks that way —
//! `Arena::units` is kept in id order and nothing here draws (doc 16 §3.5's pinned
//! draw-label set).
//!
//! A commander that becomes a wreck does not hand over: the squad keeps asking
//! through a dead tank's contact set, which is empty, so the squad drops to
//! doctrine the moment its commander dies. That is the honest reading of *the
//! fallback when nothing arrives is the reflex doctrine* — a decapitated squad
//! fights on its own reflexes.

use std::fmt;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

use crate::arena::state::Team;

/// How many units one squad holds — a pod pair.
pub const SQUAD_SIZE: usize = 2;

/// Below this many reflex units a side is one squad rather than several.
pub const MIN_UNITS_TO_SPLIT: usize = 4;

/// The most squads one side may have, so a field of three hundred cannot produce
/// a hundred and fifty commanders asking at once.
///
/// At [`SQUAD_SIZE`] two that is sixty-four tanks a side under command; the rest
/// fly doctrine, which is what they would have done before this cut existed.
pub const MAX_SQUADS_PER_SIDE: usize = 32;

/// **Which squad** — a side and an ordinal, rendered `red-1`.
///
/// It is a pair and not an opaque integer because every reader of it is a human:
/// a staff notice, a ledger line, an `@arena/status` row. `Ord` is derived so the
/// registry's maps iterate in a stable order (red before blue, then by ordinal),
/// which is what keeps two runs of the same match producing the same request
/// order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SquadId {
    /// The side.
    pub team: Team,
    /// The ordinal within the side, from `1`.
    pub index: u16,
}

impl fmt::Display for SquadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.team.as_str(), self.index)
    }
}

impl SquadId {
    /// Parse the rendered form back, or `None`.
    #[must_use]
    pub fn parse(text: &str) -> Option<SquadId> {
        let (team, index) = text.rsplit_once('-')?;
        let team = match team {
            "red" => Team::Red,
            "blue" => Team::Blue,
            _ => return None,
        };
        Some(SquadId {
            team,
            index: index.parse().ok()?,
        })
    }
}

/// **One squad's roster**, as the match record holds it and the arena is handed
/// it.
///
/// `units` is in unit-id order and `commander` is `units[0]`; both are fixed at
/// check-out. A unit that leaves the match stays on the roster — the arena skips
/// a unit it does not hold — because a roster that changed under an order would
/// make "which squad was that order for" a question with two answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SquadRoster {
    /// Which squad.
    pub id: SquadId,
    /// The unit whose contact set the trigger reads and whose contact ids an
    /// order's hint is written in.
    pub commander: ObjId,
    /// Every unit, in unit-id order, commander first.
    pub units: Vec<ObjId>,
}

impl SquadRoster {
    /// The side this squad is on.
    #[must_use]
    pub const fn team(&self) -> Team {
        self.id.team
    }

    /// Whether this squad holds `unit`.
    #[must_use]
    pub fn holds(&self, unit: ObjId) -> bool {
        self.units.contains(&unit)
    }
}

/// **Cut a match's reflex units into squads** (doc 16 §6).
///
/// `units` is `(unit, team, is_reflex)` in any order; only the reflex ones are
/// grouped, because a pilot's tank takes no order. Pilot-controlled units are
/// skipped entirely rather than commanded and refused: the refusal exists for the
/// order that arrives *after* a seat was taken, which is a race the arena settles,
/// and grouping a seated tank here would only guarantee that race happened.
///
/// The answer is deterministic — one pass in unit-id order per side — so two boots
/// of the same match produce the same squads and the same commanders.
#[must_use]
pub fn squads_of(units: &[(ObjId, Team, bool)]) -> Vec<SquadRoster> {
    let mut out = Vec::new();
    for team in Team::ALL {
        let mut side: Vec<ObjId> = units
            .iter()
            .filter(|(_, on, reflex)| *on == team && *reflex)
            .map(|(unit, _, _)| *unit)
            .collect();
        side.sort_unstable();
        side.dedup();
        if side.is_empty() {
            continue;
        }
        // Doc 16 §6 read the way the pods are claimed: pairs, unless there are too
        // few units for a pair to mean anything.
        let size = if side.len() < MIN_UNITS_TO_SPLIT {
            side.len()
        } else {
            SQUAD_SIZE
        };
        for (at, chunk) in side.chunks(size.max(1)).enumerate() {
            if at >= MAX_SQUADS_PER_SIDE {
                break;
            }
            let Some(commander) = chunk.first().copied() else {
                continue;
            };
            let Ok(index) = u16::try_from(at + 1) else {
                break;
            };
            out.push(SquadRoster {
                id: SquadId { team, index },
                commander,
                units: chunk.to_vec(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reflex(id: u32, team: Team) -> (ObjId, Team, bool) {
        (ObjId(id), team, true)
    }

    #[test]
    fn a_squad_id_round_trips_its_rendered_form() {
        let id = SquadId {
            team: Team::Blue,
            index: 3,
        };
        assert_eq!(id.to_string(), "blue-3");
        assert_eq!(SquadId::parse("blue-3"), Some(id));
        assert_eq!(SquadId::parse("green-1"), None);
        assert_eq!(SquadId::parse("blue"), None);
        assert_eq!(SquadId::parse("blue-x"), None);
    }

    #[test]
    fn a_small_side_is_one_squad_and_a_large_one_is_pairs() {
        let three = squads_of(&[
            reflex(9, Team::Red),
            reflex(4, Team::Red),
            reflex(6, Team::Red),
        ]);
        assert_eq!(three.len(), 1, "three tanks is one squad");
        assert_eq!(three[0].commander, ObjId(4), "the lowest id commands");
        assert_eq!(three[0].units, vec![ObjId(4), ObjId(6), ObjId(9)]);
        assert_eq!(three[0].id.to_string(), "red-1");

        let four = squads_of(&[
            reflex(9, Team::Red),
            reflex(4, Team::Red),
            reflex(6, Team::Red),
            reflex(11, Team::Red),
        ]);
        assert_eq!(four.len(), 2, "four tanks is two pod pairs");
        assert_eq!(four[0].units, vec![ObjId(4), ObjId(6)]);
        assert_eq!(four[1].units, vec![ObjId(9), ObjId(11)]);
        assert_eq!(four[1].commander, ObjId(9));
        assert_eq!(four[1].id.to_string(), "red-2");
    }

    #[test]
    fn the_sides_are_grouped_apart_and_pilots_are_skipped() {
        let mixed = squads_of(&[
            reflex(4, Team::Red),
            reflex(5, Team::Red),
            (ObjId(6), Team::Red, false),
            reflex(7, Team::Blue),
            reflex(8, Team::Blue),
            reflex(9, Team::Blue),
            reflex(10, Team::Blue),
        ]);
        assert_eq!(mixed.len(), 3);
        assert_eq!(mixed[0].id.team, Team::Red);
        assert!(!mixed[0].holds(ObjId(6)), "a seated tank takes no order");
        assert_eq!(mixed[1].id.to_string(), "blue-1");
        assert_eq!(mixed[2].id.to_string(), "blue-2");
        assert!(mixed[2].holds(ObjId(9)));
        assert_eq!(mixed[2].team(), Team::Blue);
    }

    #[test]
    fn the_side_is_capped_so_a_field_of_three_hundred_has_a_bounded_command() {
        let many: Vec<(ObjId, Team, bool)> =
            (0..300).map(|n| reflex(100_000 + n, Team::Red)).collect();
        let squads = squads_of(&many);
        assert_eq!(squads.len(), MAX_SQUADS_PER_SIDE);
        assert_eq!(squads[0].commander, ObjId(100_000));
        assert_eq!(
            squads[MAX_SQUADS_PER_SIDE - 1].units.len(),
            SQUAD_SIZE,
            "every squad inside the cap is a full pair"
        );
    }

    #[test]
    fn an_empty_field_has_no_squads() {
        assert!(squads_of(&[]).is_empty());
        assert!(squads_of(&[(ObjId(4), Team::Red, false)]).is_empty());
    }
}
