//! **Who is on a site's ground, and in which party** (doc 32 cut CF.2; Q-CF-4, Q-CF-5, Q-CF-11,
//! Q-CF-16, F-5).
//!
//! Everyone on the ground is admitted (Q-CF-16): every tactical unit placed directly on the
//! site's own domain, at its placement — rigs mid-drive and mid-mining included, after the
//! promotion settled them (Q-CF-31). Programme hulls on their pads are the carriers, and the
//! carrier as a unit is doc 32 cut CF.3's; until then a hull is left out by name.
//!
//! A unit's party is its detachment activation when it flies an open DT flight on this site, else
//! a party of one (the degenerate detachment, F-5). A party's **leader** is the roster's current
//! leader (doc 26), else its slot-order leader; a party of one is led by whoever sits in it, else
//! by the operator who launched it. Two parties are **Friends** when one owner stands behind both
//! — the same organizer for two rosters, the same leader for two parties of one, or a roster's
//! organizer leading a party of one (Q-CF-5); no organization or faction reputation is read in
//! this version (F-1).
//!
//! **Letters in admission order** (Q-CF-11): the parties sorted by their keys take `A`, `B`, … in
//! that order. Never by who promoted the site: the first letter would tell everyone on the ground
//! who marked whom, which is private (Q-CF-7).

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_sim::DomainId;
use smol_str::SmolStr;

use super::record::{FieldParty, FieldUnit, PartyKey};
use crate::arena::PartyId;
use crate::arena::mind::SortieId;

/// **One unit the field will admit.**
#[derive(Clone, Debug)]
pub(crate) struct Entrant {
    /// The unit.
    pub unit: ObjId,
    /// Its party's key.
    pub key: PartyKey,
    /// Who leads its party, when anyone does.
    pub leader: Option<ObjId>,
    /// Who stands behind its party: the roster's organizer, else the leader.
    pub owner: Option<ObjId>,
    /// Who sits in it now, and on which station.
    pub pilot: Option<(ObjId, SmolStr)>,
    /// Its open hull epoch.
    pub epoch: Option<SortieId>,
}

/// **The site's passive domain** (its port map's), when it is a lane site.
#[must_use]
pub(crate) fn site_domain(w: &World, entry: &CanonicalId) -> Option<DomainId> {
    let store = w.module_state::<crate::TacsimStore>()?;
    store
        .port_maps()
        .filter(|map| map.pack.as_str() == crate::definitions::content::LANE_PACK)
        .find(|map| &map.entry == entry)
        .and_then(|map| Some(DomainId(map.domain?)))
}

/// **Every unit on the site's ground**, in unit order: a tactical unit placed directly on the
/// domain that is not a programme hull (the carriers are CF.3's).
#[must_use]
pub(crate) fn on_ground(w: &World, domain: DomainId) -> Vec<ObjId> {
    let Some(store) = w.module_state::<mush_tactics::module::UnitStore>() else {
        return Vec::new();
    };
    store
        .units
        .keys()
        .copied()
        .filter(|unit| {
            matches!(
                crate::surface::act::where_is(w, *unit),
                crate::surface::act::Where::Ground { domain: on, .. } if on == domain
            )
        })
        .filter(|unit| crate::port::ship::hull_view(w, *unit).is_none())
        .collect()
}

/// **Who sits in `unit`**: the first occupied station, by station id.
#[must_use]
pub(crate) fn seated(w: &World, unit: ObjId) -> Option<(ObjId, SmolStr)> {
    let state = w.module_state::<mush_sim::SpatialState>()?;
    let host = state.assignments.get(&unit)?;
    host.seats
        .iter()
        .next()
        .map(|(station, seat)| (seat.actor, SmolStr::new(station.as_str())))
}

/// An open flight's operation, its epoch, its roster's leader and its organizer.
type FlightFacts = (
    crate::detachment::sortie::OperationId,
    Option<SortieId>,
    Option<ObjId>,
    Option<ObjId>,
);

/// **The open DT flight `unit` flies on `entry`**, as its operation and the flight's epoch.
fn flight_of(ctx: &Ctx<'_>, unit: ObjId, entry: &CanonicalId) -> Option<FlightFacts> {
    let handles = ctx
        .w
        .module_state::<crate::detachment::sortie::SortieHandles>()?;
    let head = ctx.action_state(&handles.heads, &unit).ok()??;
    let flight = ctx.action_state(&handles.flights, &head.flight).ok()??;
    if !flight.phase.open() || &flight.site != entry {
        return None;
    }
    let operation = ctx
        .action_state(&handles.operations, &flight.operation)
        .ok()
        .flatten();
    let leader = operation.and_then(|record| {
        record
            .command
            .as_ref()
            .and_then(|saved| {
                mush_tactics::detachment::roster::DetachmentRoster::restore(saved).ok()
            })
            .and_then(|roster| roster.leader().map(|leader| leader.character))
            .or_else(|| {
                record
                    .roster
                    .plan
                    .slots
                    .first()
                    .and_then(|slot| slot.controller.character())
            })
    });
    Some((
        flight.operation,
        flight.epoch.as_ref().map(|open| open.id),
        leader,
        Some(flight.operation.organizer),
    ))
}

/// **The rig's own record on the excursion family**, when it has one.
fn rig_record(ctx: &Ctx<'_>, unit: ObjId) -> Option<crate::surface::excursion::RigRecord> {
    let handles = ctx.w.module_state::<crate::port::families::PortHandles>()?;
    ctx.action_state(
        &handles.families.excursions,
        &crate::surface::excursion::RigKey(unit),
    )
    .ok()
    .flatten()
    .cloned()
}

/// **Each unit on the ground as the field admits it** (this module's head), in unit order.
#[must_use]
pub(crate) fn entrants(ctx: &Ctx<'_>, entry: &CanonicalId, units: &[ObjId]) -> Vec<Entrant> {
    units
        .iter()
        .map(|unit| {
            let pilot = seated(ctx.w, *unit);
            if let Some((operation, epoch, leader, organizer)) = flight_of(ctx, *unit, entry) {
                return Entrant {
                    unit: *unit,
                    key: PartyKey::Operation { operation },
                    leader,
                    owner: organizer,
                    pilot,
                    epoch,
                };
            }
            let record = rig_record(ctx, *unit);
            let operator = record
                .as_ref()
                .and_then(|record| record.excursion.as_ref().map(|out| out.operator));
            let leader = pilot.as_ref().map(|(actor, _)| *actor).or(operator);
            Entrant {
                unit: *unit,
                key: PartyKey::Unit { unit: *unit },
                leader,
                owner: leader,
                pilot,
                epoch: record
                    .as_ref()
                    .and_then(|record| record.sortie.as_ref().map(|open| open.id)),
            }
        })
        .collect()
}

/// **The parties the entrants form, lettered in admission order**, and each unit's row.
///
/// # Errors
///
/// More parties than the alphabet holds.
pub(crate) fn letter(
    entrants: &[Entrant],
) -> Result<(Vec<FieldParty>, Vec<FieldUnit>), &'static str> {
    let mut keys: BTreeMap<PartyKey, (Option<ObjId>, Option<ObjId>)> = BTreeMap::new();
    for entrant in entrants {
        keys.entry(entrant.key)
            .or_insert((entrant.leader, entrant.owner));
    }
    let mut letters: BTreeMap<PartyKey, PartyId> = BTreeMap::new();
    let mut parties = Vec::new();
    for (n, (key, (leader, _))) in keys.iter().enumerate() {
        let party = PartyId::nth(n).ok_or("more parties are on the ground than a field letters")?;
        letters.insert(*key, party);
        parties.push(FieldParty {
            party,
            key: *key,
            leader: *leader,
        });
    }
    parties.sort_by_key(|party| party.party);
    let mut units: Vec<FieldUnit> = entrants
        .iter()
        .filter_map(|entrant| {
            Some(FieldUnit {
                unit: entrant.unit,
                party: *letters.get(&entrant.key)?,
                pilot: entrant.pilot.clone(),
                epoch: entrant.epoch,
            })
        })
        .collect();
    units.sort_by_key(|row| row.unit);
    Ok((parties, units))
}

/// **The standing friendships** (Q-CF-5): every pair of parties one owner stands behind.
#[must_use]
pub(crate) fn friends(entrants: &[Entrant], parties: &[FieldParty]) -> Vec<(PartyId, PartyId)> {
    let letter_of = |key: &PartyKey| {
        parties
            .iter()
            .find(|party| &party.key == key)
            .map(|party| party.party)
    };
    let mut by_owner: BTreeMap<ObjId, BTreeSet<PartyId>> = BTreeMap::new();
    for entrant in entrants {
        if let (Some(owner), Some(party)) = (entrant.owner, letter_of(&entrant.key)) {
            by_owner.entry(owner).or_default().insert(party);
        }
    }
    let mut pairs = BTreeSet::new();
    for set in by_owner.values() {
        for a in set {
            for b in set {
                if a < b {
                    pairs.insert((*a, *b));
                }
            }
        }
    }
    pairs.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(unit: u32, leader: Option<u32>) -> Entrant {
        Entrant {
            unit: ObjId(unit),
            key: PartyKey::Unit { unit: ObjId(unit) },
            leader: leader.map(ObjId),
            owner: leader.map(ObjId),
            pilot: None,
            epoch: None,
        }
    }

    #[test]
    fn letters_follow_the_keys_and_never_the_promoter() {
        // Whoever marked, the letters are the keys' order.
        let entrants = vec![one(30, Some(3)), one(10, Some(1)), one(20, None)];
        let (parties, units) = letter(&entrants).expect("three parties");
        let letters: Vec<char> = parties.iter().map(|party| party.party.letter()).collect();
        assert_eq!(letters, vec!['A', 'B', 'C']);
        assert_eq!(units[0].unit, ObjId(10));
        assert_eq!(units[0].party, PartyId::nth(0).expect("A"));
        assert_eq!(units[2].party, PartyId::nth(2).expect("C"));
    }

    #[test]
    fn one_owner_makes_friends_and_a_bystander_stays_apart() {
        let entrants = vec![one(10, Some(1)), one(11, Some(1)), one(20, Some(2))];
        let (parties, _) = letter(&entrants).expect("three parties");
        let pairs = friends(&entrants, &parties);
        assert_eq!(
            pairs,
            vec![(PartyId::nth(0).expect("A"), PartyId::nth(1).expect("B"))]
        );
    }
}
