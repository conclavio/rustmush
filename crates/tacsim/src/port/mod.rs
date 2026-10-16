//! **The trade run's port code** (doc 18 cut IC2; doc 18 §4.2 "Compartments",
//! §4.3 "The capacity function", §4.4, §4.7).
//!
//! [`capacity`] is the one pure function a hull's stowage compartments are
//! checked through, everywhere a load is asked about: the market lot leg's
//! pre-check and the Draw prepare (cuts IC7/IC8), `+ship/plan` and
//! `+ship/hold`, the room gauges (IC6) and the queue's buy clamp and `fill`
//! (IC11). The port tick (cut IC5, `port_tick.rs`) and the passage planner
//! (cut IC9, `passage.rs`) are separate files beside it (doc 18 §5).
//!
//! **Doc 18 cut IC3** binds the corridor's ports into the world:
//!
//! - [`def`] — the `port` pack entry ([`PortDef`]) and its checks;
//! - [`map`] — the applier's fail-closed record of what it made, one
//!   `tacsim:port:{pack}/{entry}` document per port entry;
//! - [`apply`] — the `PortApplier`: the domain, the treasury, the orgs and
//!   their capital (doc 18 §4.7 parts a–c), once, under the content latch;
//! - [`readiness`] — the readiness report `@tacsim/port status` renders and
//!   cut IC6 gates `+coop/enrol` on;
//! - [`credit`] — `+credit`, the pilot's own account.
//!
//! **Doc 18 cut IC4** makes the programme's hulls places people stand in:
//!
//! - [`hull`] — `PortApplier` part (f): each hull Thing, its unit, its seven
//!   rooms and five exit pairs from the Dray's `interior` template, the store
//!   and airlock spaces with the airlock's member rooms and berths, the laid-up
//!   placement and the title under `tacsim.registry`; and the readiness
//!   report's hulls line.

pub mod apply;
pub mod capacity;
pub mod credit;
pub mod def;
pub mod hull;
pub mod map;
pub mod readiness;

pub use def::{
    AskDef, BidDef, MAX_TRANCHE_MAJOR, PORT_MINT_CAP_MAJOR, PORT_ROOM_ROLES, PortDef, PortOrgDef,
    ProgrammeDef, StockRowDef, VendorDef, check_ports, room_key, tranches,
};
pub use map::{
    HullRow, PORT_MAP_PREFIX, PortMap, PortMapState, PortOrgRow, TrancheRow, port_map_key,
};
pub use readiness::{Binding, LineState, Readiness, ReadinessLine, readiness};
