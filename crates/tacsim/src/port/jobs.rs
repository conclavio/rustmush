//! **The ports' job board** (doc 48 cut EX.5, Q-48-12): TacSim's answer to the market's
//! `ServiceBoard` seam. A venue's board is every open job its hosts hold, each from its own
//! record: the retrievals bound for the port, the travellers waiting there for passage, and
//! Occator's survey commissions at the programme port. Nothing is stored here.

use mush_core::World;
use mush_market::{MAX_SERVICES_SHOWN, MarketId, Service, ServiceBoard};

/// The composition's board.
#[derive(Clone, Copy, Debug, Default)]
pub struct PortJobs;

impl ServiceBoard for PortJobs {
    fn services(&self, w: &World, venue: &MarketId) -> Vec<Service> {
        let mut out = crate::surface::retrieval::services(w, venue);
        out.extend(crate::port::passenger::board::services(w, venue));
        out.extend(crate::port::survey::services(w, venue));
        // Doc 48 cut EX.6: the hauls and the escorts.
        out.extend(crate::services::haul::services(w, venue));
        out.extend(crate::services::escort::services(w, venue));
        out.truncate(MAX_SERVICES_SHOWN + 1);
        out
    }
}
