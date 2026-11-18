//! **Travellers on the job board** (doc 48 cut EX.5, Q-48-12): a traveller waiting at a port for
//! passage is a `Wanted` passage on that venue's board. The fare is the traveller's opening
//! figure, negotiated by `+passage/offer`; the passage pays on arrival.

use mush_core::{ObjKind, World};
use mush_economy::Amount;
use mush_org::PrincipalId;

use super::store::{self, Phase};

/// **The travellers waiting for passage on `venue`'s board**: each authored traveller standing in
/// the port with a funded purse and no passage booked or under way. A traveller already
/// carried never returns to the board: they stand at their destination, not their origin.
#[must_use]
pub fn services(w: &World, venue: &mush_market::MarketId) -> Vec<mush_market::Service> {
    // The bookings are a display projection; until it is ready the board shows no traveller,
    // rather than one who may already be booked.
    if !store::projection_ready(w) {
        return Vec::new();
    }
    let booked: Vec<mush_core::ObjId> = store::list_world(w)
        .into_iter()
        .filter(|row| matches!(row.phase, Phase::Booked | Phase::Aboard))
        .map(|row| row.npc)
        .collect();
    let instrument = mush_economy::InstrumentId::new(crate::economy::CREDIT);
    let mut out = Vec::new();
    for room in super::movement::station_rooms(w, venue.as_str()) {
        for npc in w.contents_of(room).iter().copied() {
            if !w.get(npc).is_some_and(|o| o.kind == ObjKind::Npc) || booked.contains(&npc) {
                continue;
            }
            let Some(def) = super::traveller(w, npc) else {
                continue;
            };
            if def.origin != venue.as_str() || !super::economy::may_accept(w, npc) {
                continue;
            }
            let name = crate::port::coop::name_of(w, npc);
            out.push(mush_market::Service {
                kind: mush_market::ServiceKind::Passage,
                side: mush_market::ServiceSide::Wanted,
                venue: venue.clone(),
                poster: PrincipalId::Object(npc),
                poster_name: name.clone().into(),
                subject: format!(
                    "carry {name} to {}",
                    crate::port::passenger::port_title(w, &def.destination)
                )
                .into(),
                pay: Amount::from_minor(def.opening_fare_minor),
                instrument: instrument.clone(),
                negotiable: true,
                bond: None,
                term: mush_market::ServiceTerm::OnPerformance,
                contract: None,
                take: format!("+passage/offer #{}", npc.0).into(),
            });
        }
    }
    out
}
