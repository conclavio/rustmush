//! **Who is on a site's ground, and in which party** (doc 32 cut CF.2; Q-CF-4, Q-CF-5, Q-CF-11,
//! Q-CF-16, F-5).
//!
//! Everyone on the ground is admitted (Q-CF-16): every tactical unit placed directly on the
//! site's own domain, at its placement — rigs mid-drive and mid-mining included, after the
//! promotion settled them (Q-CF-31). Programme hulls on their pads are the carriers, admitted as
//! units at their pad poses since doc 32 cut CF.3 (Q-CF-19, [`super::carrier`]): a carrier is in
//! the party of the roster it launched, else a party of one led by its commander.
//!
//! A unit's party is its detachment activation when it flies an open DT flight on this site, else
//! a party of one (the degenerate detachment, F-5). A party's **leader** is the roster's current
//! leader (doc 26), else its slot-order leader; a party of one is led by whoever sits in it, else
//! by the operator who launched it. Two parties are **Friends** when one owner stands behind both
//! — the same organizer for two rosters, the same leader for two parties of one, or a roster's
//! organizer leading a party of one (Q-CF-5) — or, since doc 36 cut GC.10 (Q-36-10, F-1), when
//! their leaders or owners share an organization: an active organization membership, a faction
//! membership, or a faction's principalship, each only where fellow members may know it
//! ([`affiliations`]). A raider's organization (doc 36 cut GC.11) is a faction whose principal is
//! the raider's leader, so a member of it is the raider's friend. Friends rank below a leader's
//! mark and below hostility fire produced (the kernel's [`crate::arena::party::Relations`]).
//!
//! **Declared stances** (doc 40 cut FR.1; Q-38-23, Q-40-12, §4.2): mush-faction's stance table,
//! between the same [`Affiliation`]s, read as a field is built and at a join, never mid-field
//! ([`opening_stances`]). A party whose organization declared Hostile on one of another's opens
//! the field with a Hostile mark on it, exactly as a raider's standing mark (so it beats shared
//! membership and a later Friendly mark replaces it), and the live record keeps it
//! ([`FieldParty::stance_hostile`]) so a clear returns to it. A mutual Allied pair is a friendship
//! beside the ones above (below a mark and fire); a one-sided Allied is nothing. Neutral is no row.
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
    /// Everyone physically aboard it at admission, when it is a carrier (Q-CF-38).
    pub aboard: Vec<ObjId>,
    /// **The carrier it came from and goes home to** (doc 32 cut CF.6b): its DT flight's, its
    /// rig lease's hull, or its raider's fixture carrier.
    pub carrier: Option<ObjId>,
    /// **The organizations its leader and owner openly stand in** (doc 36 cut GC.10, Q-36-10).
    pub affiliations: BTreeSet<Affiliation>,
    /// **A director raid's hull** (doc 40 cut FR.2, Q-40-18): a fixture raider's whose carrier
    /// carries the raid director's `raid` key now.
    pub director: bool,
}

/// **One organization a character openly stands in** (doc 36 cut GC.10, Q-36-10, F-1): what two
/// parties share to be Friends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Affiliation {
    /// An organization it is an active member of.
    Org(mush_org::OrgId),
    /// A faction it is a member or a principal of.
    Faction(mush_faction::FactionId),
}

/// **The organizations `who` openly stands in** (doc 36 cut GC.10): each organization it is an
/// active member of, and each faction [`mush_faction::open_factions`] names, only where fellow
/// members may know it (public or members-know). A covert membership makes nobody a friend, so a
/// field's colours never tell anyone what it hides.
#[must_use]
pub(crate) fn affiliations(w: &World, who: ObjId) -> BTreeSet<Affiliation> {
    let mut out: BTreeSet<Affiliation> = mush_faction::open_factions(w, who)
        .into_iter()
        .map(Affiliation::Faction)
        .collect();
    if let Some(store) = w.module_state::<mush_org::OrgStore>() {
        let orgs = store.state();
        out.extend(
            orgs.member_orgs(who)
                .filter(|org| {
                    orgs.membership(*org, who).is_some_and(|m| {
                        m.status == mush_org::MembershipStatus::Active
                            && matches!(
                                m.visibility,
                                mush_org::Visibility::Public | mush_org::Visibility::MembersKnow
                            )
                    })
                })
                .map(Affiliation::Org),
        );
    }
    out
}

/// The affiliations of a party's leader and owner together.
fn party_affiliations(
    w: &World,
    leader: Option<ObjId>,
    owner: Option<ObjId>,
) -> BTreeSet<Affiliation> {
    let mut out = BTreeSet::new();
    for who in [leader, owner].into_iter().flatten() {
        out.extend(affiliations(w, who));
    }
    out
}

/// **Who stands behind `party`** (doc 36 cut GC.10): its leader and, for a roster, its
/// organizer — the principals a field's standing writes name. Deduplicated, in that order.
#[must_use]
pub(crate) fn principals(party: &FieldParty) -> Vec<ObjId> {
    let mut out: Vec<ObjId> = party.leader.into_iter().collect();
    if let PartyKey::Operation { operation } = party.key
        && !out.contains(&operation.organizer)
    {
        out.push(operation.organizer);
    }
    out
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
/// domain, the carriers on their pads included (doc 32 cut CF.3, Q-CF-19).
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
        .collect()
}

/// **How many of `units` count against `[tacsim.field].max_units`** (Q-CF-21; doc 32 §15, the
/// review's CF-3a): every one but the casualties and wrecks the site record's index holds from
/// an earlier field (Q-CF-26, Q-CF-45), which are admitted all the same. Carriers count.
#[must_use]
pub(crate) fn counted(units: &[ObjId], record: &crate::surface::site_record::SiteRecord) -> usize {
    use crate::surface::site_record::LeftCondition;
    units
        .iter()
        .filter(|unit| {
            !record.units.iter().any(|left| {
                left.unit == **unit
                    && matches!(
                        left.condition,
                        LeftCondition::Casualty | LeftCondition::Wreck
                    )
            })
        })
        .count()
}

/// **How many units `site`'s ground holds against `[tacsim.field].max_units`** (Q-CF-21; doc
/// 36a §11 row 5): everyone the next promotion would admit — every tactical unit on the ground,
/// the carriers on their pads included, less the casualties a tug carries (a tow is one unit,
/// Q-36-20), the hulls held out under a licensed mind (Q-36-44), and the casualties and wrecks an
/// earlier field left ([`counted`]). The admission, the ramp's launch half
/// (`crate::surface::act::commands`) and a DT launch (`crate::detachment::sortie::prepare`) all
/// read it, so a launch refused at the ramp is exactly one the next promotion could not admit.
#[must_use]
pub(crate) fn site_population(
    w: &World,
    site: &CanonicalId,
    record: Option<&crate::surface::site_record::SiteRecord>,
) -> usize {
    let Some(domain) = site_domain(w, site) else {
        return 0;
    };
    let carried = record.map(crate::surface::tow::carried).unwrap_or_default();
    let units: Vec<ObjId> = on_ground(w, domain)
        .into_iter()
        .filter(|unit| !carried.contains_key(unit) && !super::admit::held_out(w, *unit))
        .collect();
    record.map_or(units.len(), |record| counted(&units, record))
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

/// An open flight's operation, its epoch, its roster's leader, its organizer and its carrier.
type FlightFacts = (
    crate::detachment::sortie::OperationId,
    Option<SortieId>,
    Option<ObjId>,
    Option<ObjId>,
    ObjId,
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
    let leader = ctx
        .action_state(&handles.operations, &flight.operation)
        .ok()
        .flatten()
        .and_then(roster_leader);
    Some((
        flight.operation,
        flight.epoch.as_ref().map(|open| open.id),
        leader,
        Some(flight.operation.organizer),
        flight.carrier,
    ))
}

/// **An operation's current leader** (doc 26): its command roster's, else its first slot's
/// controller (a roster that has not reconciled yet).
fn roster_leader(record: &crate::detachment::sortie::OperationRecord) -> Option<ObjId> {
    record
        .command
        .as_ref()
        .and_then(|saved| mush_tactics::detachment::roster::DetachmentRoster::restore(saved).ok())
        .and_then(|roster| roster.leader().map(|leader| leader.character))
        .or_else(|| {
            record
                .roster
                .plan
                .slots
                .first()
                .and_then(|slot| slot.controller.character())
        })
}

/// **Who leads `party` now** (doc 36a §11 row 6): a roster party's leader is its operation's
/// current command roster, read at command time as [`entrants`] reads it at admission, so a
/// succession (doc 26) moves the mark with it — the successor marks, the deposed leader does
/// not. A party of one, or a fixture raider's, keeps the leader its row names. An operation
/// record that will not read keeps the row's leader too.
#[must_use]
pub(crate) fn current_leader(ctx: &Ctx<'_>, party: &FieldParty) -> Option<ObjId> {
    match party.key {
        PartyKey::Operation { operation } => ctx
            .w
            .module_state::<crate::detachment::sortie::SortieHandles>()
            .and_then(|handles| {
                ctx.action_state(&handles.operations, &operation)
                    .ok()
                    .flatten()
            })
            .map_or(party.leader, roster_leader),
        PartyKey::Unit { .. } | PartyKey::Fixture { .. } => party.leader,
    }
}

/// **The party `actor` leads in `live` now** ([`current_leader`] over the record's parties).
#[must_use]
pub(crate) fn led_now(ctx: &Ctx<'_>, live: &super::FieldLive, actor: ObjId) -> Option<PartyId> {
    live.parties
        .iter()
        .find(|party| current_leader(ctx, party) == Some(actor))
        .map(|party| party.party)
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

/// **Each unit on the ground as the field admits it** (this module's head), in unit order. A
/// carrier takes the party of the first open flight on this site it launched (by operation),
/// else a party of one led by its commander (doc 32 cut CF.3).
#[must_use]
pub(crate) fn entrants(ctx: &Ctx<'_>, entry: &CanonicalId, units: &[ObjId]) -> Vec<Entrant> {
    let flights: Vec<(ObjId, FlightFacts)> = units
        .iter()
        .filter_map(|unit| flight_of(ctx, *unit, entry).map(|facts| (*unit, facts)))
        .collect();
    units
        .iter()
        .map(|unit| {
            if super::carrier::is_carrier(ctx.w, *unit) {
                return carrier_entrant(ctx, *unit, &flights);
            }
            let pilot = seated(ctx.w, *unit);
            if let Some((_, (operation, epoch, leader, organizer, carrier))) =
                flights.iter().find(|(flier, _)| flier == unit)
            {
                return Entrant {
                    unit: *unit,
                    key: PartyKey::Operation {
                        operation: *operation,
                    },
                    leader: *leader,
                    owner: *organizer,
                    pilot,
                    epoch: *epoch,
                    aboard: Vec::new(),
                    carrier: Some(*carrier),
                    affiliations: party_affiliations(ctx.w, *leader, *organizer),
                    director: false,
                };
            }
            // A fixture raider's hull: its leader's party (doc 32 cut CF.6, Q-CF-44).
            if let Some(leader) = super::raiders::leader_of(ctx.w, *unit) {
                return Entrant {
                    unit: *unit,
                    key: PartyKey::Fixture { leader },
                    leader: Some(leader),
                    owner: Some(leader),
                    pilot,
                    epoch: None,
                    aboard: Vec::new(),
                    carrier: super::raiders::carrier_of(ctx.w, *unit),
                    affiliations: affiliations(ctx.w, leader),
                    director: super::raiders::carrier_of(ctx.w, *unit)
                        .is_some_and(|carrier| crate::surface::raid::director_raid(ctx.w, carrier)),
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
                aboard: Vec::new(),
                carrier: record
                    .as_ref()
                    .and_then(|record| record.excursion.as_ref().map(|out| out.hull)),
                affiliations: party_affiliations(ctx.w, leader, None),
                director: false,
            }
        })
        .collect()
}

/// **A carrier as the field admits it** (Q-CF-19, Q-CF-38): the roster it launched, else its
/// commander's party of one; no pilot (its helm is not a cockpit); its own hull epoch; everyone
/// aboard.
fn carrier_entrant(ctx: &Ctx<'_>, carrier: ObjId, flights: &[(ObjId, FlightFacts)]) -> Entrant {
    let roster = flights
        .iter()
        .map(|(_, facts)| facts)
        .filter(|facts| facts.4 == carrier)
        .min_by_key(|facts| facts.0);
    let commander = crate::port::coop::helm_holder(ctx.w, carrier);
    let (key, leader, owner) = match roster {
        Some((operation, _, leader, organizer, _)) => (
            PartyKey::Operation {
                operation: *operation,
            },
            *leader,
            *organizer,
        ),
        None => (PartyKey::Unit { unit: carrier }, commander, commander),
    };
    Entrant {
        unit: carrier,
        key,
        leader,
        owner,
        pilot: None,
        epoch: super::carrier::open_epoch(ctx, carrier),
        aboard: super::carrier::souls_aboard(ctx.w, carrier),
        carrier: None,
        affiliations: party_affiliations(ctx.w, leader, owner),
        director: false,
    }
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
            stance_hostile: Vec::new(),
            director: entrants
                .iter()
                .any(|entrant| entrant.key == *key && entrant.director),
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
                aboard: entrant.aboard.clone(),
                arrived: false,
            })
        })
        .collect();
    units.sort_by_key(|row| row.unit);
    Ok((parties, units))
}

/// What two parties may share to be Friends: an owner, or an organization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Tie {
    Owner(ObjId),
    Org(Affiliation),
}

/// Every pair of parties some tie holds together, each pair once, lower letter first.
fn pair_up(ties: impl IntoIterator<Item = (Tie, PartyId)>) -> Vec<(PartyId, PartyId)> {
    let mut by_tie: BTreeMap<Tie, BTreeSet<PartyId>> = BTreeMap::new();
    for (tie, party) in ties {
        by_tie.entry(tie).or_default().insert(party);
    }
    let mut pairs = BTreeSet::new();
    for set in by_tie.values() {
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

/// **The standing friendships** (Q-CF-5; doc 36 cut GC.10, Q-36-10): every pair of parties one
/// owner stands behind, or whose leaders or owners share an organization ([`affiliations`]).
#[must_use]
pub(crate) fn friends(entrants: &[Entrant], parties: &[FieldParty]) -> Vec<(PartyId, PartyId)> {
    let letter_of = |key: &PartyKey| {
        parties
            .iter()
            .find(|party| &party.key == key)
            .map(|party| party.party)
    };
    let mut ties = Vec::new();
    for entrant in entrants {
        let Some(party) = letter_of(&entrant.key) else {
            continue;
        };
        if let Some(owner) = entrant.owner {
            ties.push((Tie::Owner(owner), party));
        }
        ties.extend(
            entrant
                .affiliations
                .iter()
                .map(|affiliation| (Tie::Org(*affiliation), party)),
        );
    }
    pair_up(ties)
}

/// **The standing friendships of a field's parties, read now** (doc 36 cut GC.10): [`friends`]'s
/// rule over the live record's parties, each owned by its leader or, for a roster, its organizer,
/// and since doc 40 cut FR.1 every mutually Allied pair (Q-40-14: the void counts it as it counts
/// shared membership). The demotion reads it to tell friendly fire.
#[must_use]
pub(crate) fn friends_of(w: &World, parties: &[FieldParty]) -> Vec<(PartyId, PartyId)> {
    let mut ties = Vec::new();
    let mut sides: BTreeMap<PartyId, BTreeSet<Affiliation>> = BTreeMap::new();
    for party in parties {
        let owner = match party.key {
            PartyKey::Operation { operation } => Some(operation.organizer),
            PartyKey::Unit { .. } | PartyKey::Fixture { .. } => party.leader,
        };
        if let Some(owner) = owner {
            ties.push((Tie::Owner(owner), party.party));
        }
        let affiliations = party_affiliations(w, party.leader, owner);
        ties.extend(
            affiliations
                .iter()
                .map(|affiliation| (Tie::Org(*affiliation), party.party)),
        );
        sides.insert(party.party, affiliations);
    }
    let mut pairs: BTreeSet<(PartyId, PartyId)> = pair_up(ties).into_iter().collect();
    pairs.extend(declared(w, &sides).1);
    pairs.into_iter().collect()
}

/// An organization as a side of mush-faction's stance table.
fn stance_side(affiliation: Affiliation) -> mush_faction::StanceSide {
    match affiliation {
        Affiliation::Org(org) => mush_faction::StanceSide::Org(org),
        Affiliation::Faction(faction) => mush_faction::StanceSide::Faction(faction),
    }
}

/// **Each party's organizations**: the union of its entrants' [`affiliations`].
fn party_sides(
    entrants: &[Entrant],
    parties: &[FieldParty],
) -> BTreeMap<PartyId, BTreeSet<Affiliation>> {
    let mut sides: BTreeMap<PartyId, BTreeSet<Affiliation>> = parties
        .iter()
        .map(|party| (party.party, BTreeSet::new()))
        .collect();
    for entrant in entrants {
        if let Some(party) = parties.iter().find(|party| party.key == entrant.key) {
            sides
                .entry(party.party)
                .or_default()
                .extend(entrant.affiliations.iter().copied());
        }
    }
    sides
}

/// A set of party pairs.
type Pairs = BTreeSet<(PartyId, PartyId)>;

/// **The declared stances between `sides`, read now** (doc 40 cut FR.1; Q-40-12): every ordered
/// pair `(by, of)` where an organization of `by` declared Hostile on one of `of`'s, and every pair
/// some organization of each holds mutually Allied with one of the other's, lower letter first.
fn declared(w: &World, sides: &BTreeMap<PartyId, BTreeSet<Affiliation>>) -> (Pairs, Pairs) {
    let mut hostile = BTreeSet::new();
    let mut allied = BTreeSet::new();
    let Some(table) = w
        .module_state::<mush_faction::FactionState>()
        .filter(|table| !table.stances.is_empty())
    else {
        return (hostile, allied);
    };
    for (by, mine) in sides {
        for (of, theirs) in sides {
            if by == of {
                continue;
            }
            let any =
                |held: &dyn Fn(mush_faction::StanceSide, mush_faction::StanceSide) -> bool| {
                    mine.iter().any(|a| {
                        theirs
                            .iter()
                            .any(|b| held(stance_side(*a), stance_side(*b)))
                    })
                };
            if any(&|a, b| table.stance(a, b) == mush_faction::Stance::Hostile) {
                hostile.insert((*by, *of));
            }
            if by < of && any(&|a, b| table.mutually_allied(a, b)) {
                allied.insert((*by, *of));
            }
        }
    }
    (hostile, allied)
}

/// **What a field opens with by standing word** (doc 40 cut FR.1, §4.2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OpeningStances {
    /// Every `(by, of)`: `by`'s leader opens marking `of` Hostile — `by`'s organizations declared
    /// Hostile on `of`'s, or `by` is a fixture raider (Q-CF-44).
    pub hostile: Vec<(PartyId, PartyId)>,
    /// Every mutually Allied pair, lower letter first: a friendship (Q-40-12).
    pub allied: Vec<(PartyId, PartyId)>,
}

/// **The stance table read over `parties` now** (doc 40 cut FR.1; Q-38-23): at admission over
/// every party, at a join for the pairs the joining party opens.
#[must_use]
pub(crate) fn opening_stances(
    w: &World,
    entrants: &[Entrant],
    parties: &[FieldParty],
) -> OpeningStances {
    let (mut hostile, allied) = declared(w, &party_sides(entrants, parties));
    // A fixture raider's leader holds a standing Hostile mark on every other party (doc 32 cut
    // CF.6, Q-CF-44): the raider fights whoever it finds, with no player connected.
    for raider in parties.iter().filter(|party| party.key.is_fixture()) {
        for other in parties.iter().filter(|other| other.party != raider.party) {
            hostile.insert((raider.party, other.party));
        }
    }
    OpeningStances {
        hostile: hostile.into_iter().collect(),
        allied: allied.into_iter().collect(),
    }
}

/// **The opening Hostile marks a live record's `stance_hostile` rows hold that name `party`**
/// (doc 40 cut FR.1), as the kernel's `(by, of, mark)` rows.
#[must_use]
pub(crate) fn standing_marks(
    parties: &[FieldParty],
    party: Option<PartyId>,
) -> Vec<(PartyId, PartyId, crate::arena::party::Mark)> {
    parties
        .iter()
        .flat_map(|by| {
            by.stance_hostile
                .iter()
                .map(move |of| (by.party, *of, crate::arena::party::Mark::Hostile))
        })
        .filter(|(by, of, _)| party.is_none_or(|party| *by == party || *of == party))
        .collect()
}

/// **The parties on `site`'s ground as a field would admit them now, and which are Friends**
/// (doc 36 cut GC.10; Q-36-10): the presence picture's parties, lettered, with every standing
/// friendship among them — since doc 40 cut FR.1 each mutually Allied pair too, and each party's
/// `stance_hostile` as admission would write it. Staff and the gates read it; nothing is written.
#[must_use]
pub fn friends_on_ground(
    ctx: &Ctx<'_>,
    site: &CanonicalId,
) -> (Vec<FieldParty>, Vec<(PartyId, PartyId)>) {
    let Some(domain) = site_domain(ctx.w, site) else {
        return (Vec::new(), Vec::new());
    };
    let units = on_ground(ctx.w, domain);
    let entrants = entrants(ctx, site, &units);
    let Ok((mut parties, _)) = letter(&entrants) else {
        return (Vec::new(), Vec::new());
    };
    let allied = admit_stances(ctx.w, &entrants, &mut parties);
    let mut pairs = friends(&entrants, &parties);
    for pair in allied {
        if !pairs.contains(&pair) {
            pairs.push(pair);
        }
    }
    pairs.sort_unstable();
    (parties, pairs)
}

/// **Write admission's standing Hostile marks into `parties`** (doc 40 cut FR.1, §4.2) — each
/// party's `stance_hostile`, from [`opening_stances`] — and return its mutually Allied pairs.
pub(crate) fn admit_stances(
    w: &World,
    entrants: &[Entrant],
    parties: &mut [FieldParty],
) -> Vec<(PartyId, PartyId)> {
    let stances = opening_stances(w, entrants, parties);
    for party in parties.iter_mut() {
        party.stance_hostile = stances
            .hostile
            .iter()
            .filter(|(by, _)| *by == party.party)
            .map(|(_, of)| *of)
            .collect();
    }
    stances.allied
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
            aboard: Vec::new(),
            carrier: None,
            affiliations: BTreeSet::new(),
            director: false,
        }
    }

    #[test]
    fn one_organization_makes_friends_across_owners_and_a_third_party_stays_apart() {
        // Doc 36 cut GC.10 (Q-36-10): three owners; the first two share a faction, the third a
        // different one. Only the first pair are friends.
        let shared = Affiliation::Faction(mush_faction::FactionId(7));
        let other = Affiliation::Faction(mush_faction::FactionId(8));
        let mut entrants = vec![one(10, Some(1)), one(20, Some(2)), one(30, Some(3))];
        entrants[0].affiliations.insert(shared);
        entrants[1].affiliations.insert(shared);
        entrants[2].affiliations.insert(other);
        let (parties, _) = letter(&entrants).expect("three parties");
        assert_eq!(
            friends(&entrants, &parties),
            vec![(PartyId::nth(0).expect("A"), PartyId::nth(1).expect("B"))]
        );
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
