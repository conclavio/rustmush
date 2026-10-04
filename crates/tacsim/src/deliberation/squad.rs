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
//! A side's reflex units, in unit-id order, **dealt** into squads of at most
//! `[tacsim.arena].squad_size` units (16b cut Z.1, 16c N6: *lances*; four by
//! default, [`SQUAD_SIZE_MAX`] at most): `ceil(n / size)` squads whose sizes differ
//! by at most one, the larger ones first, each a run of consecutive ids. Twelve
//! machines a side is an opponent only in squads of four; a side of at most `size`
//! machines is one squad by the formula, with no special case.
//!
//! **Dealt, not chunked**, because `SIM AI 6` with one human puts five machines on
//! red, and chunking by four would make a four and a one — a lone tank with a
//! commander of its own, asking for orders as often as the whole lance beside it.
//! Dealing makes a three and a two. Past [`MAX_SQUADS_PER_SIDE`] squads of `size` a
//! side stops dealing and fills the cap with full squads; the rest fly doctrine
//! (16b cut N.2's cap, unchanged in meaning).
//!
//! Both callers read the size from the [`crate::config::ArenaConfig`] each
//! already holds — the match record's grouping at `crate::match_::start` and the
//! arena's own at `Arena::new` — so the two groupings of one match are one function
//! of one data and cannot drift.
//!
//! The **commander** is the lowest unit id in the squad. It is still not a
//! capability and not a station — it grants nothing a seat could take — but since
//! **17b cut L.1** (doc 17 §4 "Where a lance-leader sits", D-UM-53) it is **where
//! the lance-leader rides**: the squad's leader is a character (an AI-played
//! character attached to the squad, or in the OOC sim the persona
//! [`fill_leaders`] mints per squad), never a `crate::arena::state::Controller`
//! value — D-UM-2: a *seated pilot* is a seat activated by a player's connection,
//! and the leader is not one. The commander is the hull whose contact set the
//! trigger reads and whose opaque contact ids a leader's target hint is written in
//! (today's `issuer: None` scope, unchanged); the order's **author** is the leader,
//! in `crate::deliberation::order::Order::by`, never the unit. It is the lowest id
//! for the same reason every other tie in this engine breaks that way —
//! `Arena::units` is kept in id order and nothing here draws (doc 16 §3.5's pinned
//! draw-label set).
//!
//! A commander that becomes a wreck does not hand over: the leader is down with
//! its hull, the squad stops asking (`crate::arena::command`'s trigger skips a
//! wrecked commander), and the squad drops to doctrine the moment its commander
//! dies. That is the honest reading of *the fallback when nothing arrives is the
//! reflex doctrine* — a decapitated squad fights on its own reflexes.

use std::fmt;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

use crate::arena::state::Team;
use crate::deliberation::order::CharacterRef;

/// **The most `[tacsim.arena].squad_size` may be** (16b cut Z.1, 16c N6), and
/// the clamp [`squads_of`] reads a size through.
///
/// Sixteen, because a squad is one commander's contact set and one order: past
/// sixteen machines an order stops being about a squad and starts being about a
/// side, which is the lead's job and not one prompt's.
pub const SQUAD_SIZE_MAX: u32 = 16;

/// The most squads one side may have, so a field of three hundred cannot produce
/// a hundred and fifty commanders asking at once.
///
/// At the shipped `squad_size` of four that is a hundred and twenty-eight tanks a
/// side under command; the rest fly doctrine, which is what they would have done
/// before cut N.2 existed.
pub const MAX_SQUADS_PER_SIDE: usize = 32;

/// **Which squad** — a side and an ordinal, rendered `red-1`.
///
/// It is a pair and not an opaque integer because every reader of it is a human:
/// a staff notice, a ledger line, an `@arena/status` row. `Ord` is derived so the
/// registry's maps iterate in a stable order (red before blue, then by ordinal),
/// which is what keeps two runs of the same match producing the same request
/// order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
/// `size` is `[tacsim.arena].squad_size` (16b cut Z.1), read through
/// `clamp(1, SQUAD_SIZE_MAX)` so a caller holding an unvalidated config still gets
/// squads. Per side, with `n` reflex units, in this order:
///
/// 1. `n == 0`: no squad;
/// 2. `n <= MAX_SQUADS_PER_SIDE × size`: `ceil(n / size)` squads, dealt — squad
///    `k` takes `n / squads + 1` units while `k < n % squads` and `n / squads`
///    after, consecutive in id order — so at size four one to four machines are
///    one squad, five are a three and a two, six two threes, twelve three fours;
/// 3. otherwise: [`MAX_SQUADS_PER_SIDE`] squads of exactly `size`, and the rest in
///    no squad.
///
/// The answer is deterministic — one pass in unit-id order per side — so two boots
/// of the same match produce the same squads and the same commanders.
#[must_use]
pub fn squads_of(units: &[(ObjId, Team, bool)], size: u32) -> Vec<SquadRoster> {
    let size = usize::try_from(size.clamp(1, SQUAD_SIZE_MAX)).unwrap_or(1);
    let mut out = Vec::new();
    for team in Team::ALL {
        let mut side: Vec<ObjId> = units
            .iter()
            .filter(|(_, on, reflex)| *on == team && *reflex)
            .map(|(unit, _, _)| *unit)
            .collect();
        side.sort_unstable();
        side.dedup();
        let n = side.len();
        if n == 0 {
            continue;
        }
        // **16b cut Z.1**: deal, don't chunk. Inside the cap every squad is `base`
        // or `base + 1` units and the larger come first; past it every squad is a
        // full `size` and the cap decides where the command stops.
        let (squads, base, extra) = if n <= MAX_SQUADS_PER_SIDE.saturating_mul(size) {
            let squads = n.div_ceil(size);
            (squads, n / squads, n % squads)
        } else {
            (MAX_SQUADS_PER_SIDE, size, 0)
        };
        let mut from = 0_usize;
        for at in 0..squads {
            let take = if at < extra { base + 1 } else { base };
            let Some(dealt) = side.get(from..from.saturating_add(take)) else {
                break;
            };
            from = from.saturating_add(take);
            let Some(commander) = dealt.first().copied() else {
                continue;
            };
            let Ok(index) = u16::try_from(at + 1) else {
                break;
            };
            out.push(SquadRoster {
                id: SquadId { team, index },
                commander,
                units: dealt.to_vec(),
            });
        }
    }
    out
}

/// **A squad's lance-leader** (17b cut L.1; doc 17 §4, D-UM-53) — the character
/// principal who issues the squad's deliberated `ORDER`s, riding the squad's
/// commander hull.
///
/// The match record holds these beside its rosters; the arena holds none (a
/// [`SquadRoster`] carries no principal). A squad with no row has no leader and
/// runs on doctrine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanceLeader {
    /// The squad led.
    pub squad: SquadId,
    /// Who leads it.
    pub by: CharacterRef,
}

/// **Mint the OOC sim's lance-leaders** (D-UM-53: "mints one leader per AI fill
/// squad"): one [`CharacterRef::Persona`] per roster, in roster order.
///
/// Every squad [`squads_of`] deals in the OOC sim is a fill squad of machines and
/// deliberates today, so every one of them gets a leader and nothing that asked
/// before stops asking.
#[must_use]
pub fn fill_leaders(squads: &[SquadRoster]) -> Vec<LanceLeader> {
    squads
        .iter()
        .map(|roster| LanceLeader {
            squad: roster.id,
            by: CharacterRef::Persona { squad: roster.id },
        })
        .collect()
}

/// **Who leads `squad`**, or `None` for a squad on doctrine.
#[must_use]
pub fn leader_of(leaders: &[LanceLeader], squad: SquadId) -> Option<&CharacterRef> {
    leaders
        .iter()
        .find(|leader| leader.squad == squad)
        .map(|leader| &leader.by)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **17b cut L.1 (D-UM-53)**: one persona per roster, in roster order, and the
    /// lookup finds a squad's leader or none.
    #[test]
    fn l1_fill_leaders_mints_one_persona_per_roster_and_leader_of_finds_it() {
        let squads = squads_of(
            &[
                reflex(4, Team::Red),
                reflex(5, Team::Red),
                reflex(7, Team::Blue),
            ],
            1,
        );
        let leaders = fill_leaders(&squads);
        assert_eq!(leaders.len(), squads.len());
        for (leader, roster) in leaders.iter().zip(&squads) {
            assert_eq!(leader.squad, roster.id);
            assert_eq!(leader.by, CharacterRef::Persona { squad: roster.id });
        }
        let red_two = SquadId {
            team: Team::Red,
            index: 2,
        };
        assert_eq!(
            leader_of(&leaders, red_two),
            Some(&CharacterRef::Persona { squad: red_two })
        );
        let blue_two = SquadId {
            team: Team::Blue,
            index: 2,
        };
        assert_eq!(leader_of(&leaders, blue_two), None, "no such squad");
        assert_eq!(leader_of(&[], red_two), None, "a squad with no leader");
        assert!(fill_leaders(&[]).is_empty());
    }

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

    /// The sizes of one side's squads, in ordinal order.
    fn sizes(squads: &[SquadRoster]) -> Vec<usize> {
        squads.iter().map(|squad| squad.units.len()).collect()
    }

    /// `n` red machines with ids `10..10 + n`.
    fn side_of(n: u32) -> Vec<(ObjId, Team, bool)> {
        (0..n).map(|at| reflex(10 + at, Team::Red)).collect()
    }

    /// **16b cut Z.1 (16c N6): a side is dealt, not chunked.** Rewrites cut N.2's
    /// `a_small_side_is_one_squad_and_a_large_one_is_pairs`, which pinned three
    /// machines as one squad and four as two pairs; at the shipped size of four,
    /// four machines are one lance, and the size-two half of the table is where
    /// the old pairs still live.
    #[test]
    fn z1_a_side_is_dealt_into_squads_whose_sizes_differ_by_at_most_one() {
        let three = squads_of(
            &[
                reflex(9, Team::Red),
                reflex(4, Team::Red),
                reflex(6, Team::Red),
            ],
            4,
        );
        assert_eq!(three.len(), 1, "three tanks is one squad");
        assert_eq!(three[0].commander, ObjId(4), "the lowest id commands");
        assert_eq!(three[0].units, vec![ObjId(4), ObjId(6), ObjId(9)]);
        assert_eq!(three[0].id.to_string(), "red-1");

        // The worked table at the shipped size.
        for (n, want) in [
            (1, vec![1]),
            (3, vec![3]),
            (4, vec![4]),
            (5, vec![3, 2]),
            (6, vec![3, 3]),
            (8, vec![4, 4]),
            (12, vec![4, 4, 4]),
        ] {
            assert_eq!(sizes(&squads_of(&side_of(n), 4)), want, "{n} at size 4");
        }
        // And at two, where cut N.2's pod pairs were.
        for (n, want) in [(3, vec![2, 1]), (4, vec![2, 2]), (5, vec![2, 2, 1])] {
            assert_eq!(sizes(&squads_of(&side_of(n), 2)), want, "{n} at size 2");
        }

        // Five on red is a three and a two, consecutive in id order, each
        // commanded by its own lowest id -- never a four and a one.
        let five = squads_of(&side_of(5), 4);
        assert_eq!(five[0].units, vec![ObjId(10), ObjId(11), ObjId(12)]);
        assert_eq!(five[1].units, vec![ObjId(13), ObjId(14)]);
        assert_eq!(
            (five[0].commander, five[1].commander),
            (ObjId(10), ObjId(13))
        );
        assert_eq!(five[1].id.to_string(), "red-2");

        // A size nobody validated is read through the clamp, not refused.
        assert_eq!(sizes(&squads_of(&side_of(3), 0)), vec![1, 1, 1]);
        assert_eq!(sizes(&squads_of(&side_of(20), 99)), vec![10, 10]);
    }

    #[test]
    fn the_sides_are_grouped_apart_and_pilots_are_skipped() {
        // Size two, so blue's four are two squads and the ordinals are per side.
        let mixed = squads_of(
            &[
                reflex(4, Team::Red),
                reflex(5, Team::Red),
                (ObjId(6), Team::Red, false),
                reflex(7, Team::Blue),
                reflex(8, Team::Blue),
                reflex(9, Team::Blue),
                reflex(10, Team::Blue),
            ],
            2,
        );
        assert_eq!(mixed.len(), 3);
        assert_eq!(mixed[0].id.team, Team::Red);
        assert!(!mixed[0].holds(ObjId(6)), "a seated tank takes no order");
        assert_eq!(mixed[1].id.to_string(), "blue-1");
        assert_eq!(mixed[2].id.to_string(), "blue-2");
        assert!(mixed[2].holds(ObjId(9)));
        assert_eq!(mixed[2].team(), Team::Blue);

        // **16b cut Z.1**: at the shipped size the pilot's tank is still skipped
        // (red's two machines are one squad of two, not three), and each side is
        // its own `red-1` / `blue-1`.
        let lances = squads_of(
            &[
                reflex(4, Team::Red),
                (ObjId(5), Team::Red, false),
                reflex(6, Team::Red),
                reflex(7, Team::Blue),
                reflex(8, Team::Blue),
                reflex(9, Team::Blue),
                reflex(10, Team::Blue),
                reflex(11, Team::Blue),
            ],
            4,
        );
        assert_eq!(sizes(&lances), vec![2, 3, 2]);
        assert_eq!(lances[0].units, vec![ObjId(4), ObjId(6)]);
        assert_eq!(lances[1].id.to_string(), "blue-1");
        assert_eq!(lances[2].id.to_string(), "blue-2");
        assert_eq!(lances[2].commander, ObjId(10));
    }

    #[test]
    fn the_side_is_capped_so_a_field_of_three_hundred_has_a_bounded_command() {
        let many: Vec<(ObjId, Team, bool)> =
            (0..300).map(|n| reflex(100_000 + n, Team::Red)).collect();
        let squads = squads_of(&many, 4);
        assert_eq!(squads.len(), MAX_SQUADS_PER_SIDE);
        assert_eq!(squads[0].commander, ObjId(100_000));
        assert!(
            squads.iter().all(|squad| squad.units.len() == 4),
            "every squad inside the cap is a full lance"
        );
        assert_eq!(
            squads[MAX_SQUADS_PER_SIDE - 1].units.last(),
            Some(&ObjId(100_127))
        );
        assert!(
            squads.iter().all(|squad| !squad.holds(ObjId(100_128))),
            "the hundred and twenty-ninth machine flies doctrine"
        );
        // And 150 a side (the CI cohort) deals into more than the cap, so it is
        // still 32 squads of four.
        let cohort: Vec<(ObjId, Team, bool)> =
            (0..150).map(|n| reflex(200_000 + n, Team::Blue)).collect();
        assert_eq!(sizes(&squads_of(&cohort, 4)), vec![4; MAX_SQUADS_PER_SIDE]);
    }

    /// **16b cut Z.1: the dealing rule, over every side size and every legal
    /// squad size** -- exhaustive rather than sampled, because the space is 3,200
    /// cases and each is a sort of at most two hundred ids.
    #[test]
    fn z1_every_squad_is_at_most_size_and_the_sizes_differ_by_at_most_one() {
        for n in 1..=200_u32 {
            let side = side_of(n);
            let ids: Vec<ObjId> = side.iter().map(|(unit, _, _)| *unit).collect();
            for size in 1..=SQUAD_SIZE_MAX {
                let squads = squads_of(&side, size);
                let cap = usize::try_from(size).unwrap_or(1);
                let lens = sizes(&squads);
                assert!(
                    lens.iter().all(|len| *len >= 1 && *len <= cap),
                    "n {n} size {size}: {lens:?}"
                );
                let low = lens.iter().min().copied().unwrap_or(0);
                let high = lens.iter().max().copied().unwrap_or(0);
                assert!(high - low <= 1, "n {n} size {size}: {lens:?}");
                assert!(squads.len() <= MAX_SQUADS_PER_SIDE);
                let mut seen: Vec<ObjId> = squads
                    .iter()
                    .flat_map(|squad| squad.units.iter().copied())
                    .collect();
                let dealt = seen.len();
                seen.sort_unstable();
                seen.dedup();
                assert_eq!(seen.len(), dealt, "n {n} size {size}: a unit twice");
                for squad in &squads {
                    assert_eq!(squad.units.first(), Some(&squad.commander));
                }
                let n_units = usize::try_from(n).unwrap_or(0);
                if n_units <= MAX_SQUADS_PER_SIDE * cap {
                    assert_eq!(
                        seen, ids,
                        "n {n} size {size}: inside the cap every unit is dealt once"
                    );
                    assert_eq!(squads.len(), n_units.div_ceil(cap));
                }
            }
        }
    }

    #[test]
    fn an_empty_field_has_no_squads() {
        assert!(squads_of(&[], 4).is_empty());
        assert!(squads_of(&[(ObjId(4), Team::Red, false)], 4).is_empty());
    }
}
