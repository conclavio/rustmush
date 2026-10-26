use super::LaunchVenue;
use mush_core::{Flags, ObjId, ObjKind, World};
use mush_inventory::EndpointRef;
use mush_market::{MarketId, Presence, VenueGate, VenueRefusal};
use mush_org::PrincipalId;

/// A real configured counter, layered over the complete ordinary port gate.
#[derive(Debug)]
pub(super) struct Gate(pub(super) LaunchVenue);
impl VenueGate for Gate {
    fn venue_of_room(&self, w: &World, room: ObjId) -> Option<MarketId> {
        if room == self.0.room
            && w.get(room)
                .is_some_and(|o| o.kind == ObjKind::Room && !o.flags.contains(Flags::GOING))
        {
            Some(self.0.name.clone())
        } else {
            crate::port::market::PortVenueGate.venue_of_room(w, room)
        }
    }
    fn endpoint_for(
        &self,
        w: &World,
        actor: ObjId,
        venue: &MarketId,
        presence: Presence,
    ) -> Option<EndpointRef> {
        crate::port::market::PortVenueGate.endpoint_for(w, actor, venue, presence)
    }
    fn admit(
        &self,
        w: &World,
        actor: ObjId,
        venue: &MarketId,
        endpoint: Option<EndpointRef>,
        presence: Presence,
    ) -> Result<(), VenueRefusal> {
        // Existing governed endpoints always retain the native port predicate.
        crate::port::market::PortVenueGate.admit(w, actor, venue, endpoint, presence)?;
        if venue == &self.0.name
            && (presence != Presence::Here
                || w.get(actor).and_then(|o| o.location) != Some(self.0.room)
                || self.venue_of_room(w, self.0.room).as_ref() != Some(venue))
        {
            return Err(VenueRefusal {
                line: "Visit the configured mind catalogue counter.".into(),
            });
        }
        Ok(())
    }
    fn governed(&self, w: &World, endpoint: EndpointRef) -> bool {
        crate::port::market::PortVenueGate.governed(w, endpoint)
    }
    fn port_seller(&self, w: &World, venue: &MarketId, seller: &PrincipalId) -> bool {
        crate::port::market::PortVenueGate.port_seller(w, venue, seller)
    }
    fn signatory(&self, w: &World, venue: &MarketId, seller: &PrincipalId) -> Option<String> {
        crate::port::market::PortVenueGate.signatory(w, venue, seller)
    }
}
