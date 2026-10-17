//! **The trade run's port code** (doc 18 cut IC2; doc 18 §4.2 "Compartments",
//! §4.3 "The capacity function", §4.4, §4.7).
//!
//! [`capacity`] is the one pure function a hull's stowage compartments are
//! checked through, everywhere a load is asked about: the market lot leg's
//! pre-check and the Draw prepare (cuts IC7/IC8), `+ship/plan` and
//! `+ship/hold`, the room gauges (IC6) and the queue's buy clamp and `fill`
//! (IC11). The port tick (cut IC5, [`crate::port_tick`], at the crate root per
//! doc 18 §5's lane note) and the passage planner (cut IC9, `passage.rs`) are
//! separate files (doc 18 §5).
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
//!
//! Doc 18 cut IC5 — [`families`]: the stock-issue receipt and the departure
//! ticket, with the `PassagePlan` they freeze; [`crate::port_tick`] is the tick.
//!
//! **Doc 18 cut IC6** — the Co-op's First Captain programme and boarding:
//!
//! - [`coop`] — `+coop`, `+coop/enrol` and `+coop/statement`; the enrolment
//!   document (one per gateway account, ever, D-18-25), the operate gate
//!   ([`live_charter`]: the live lease, never the document, D-18-23), the
//!   free-hull rule and the observer that folds a staff-ended lease;
//! - [`gauges`] — the pure gauge renders over the capacity function (no pack
//!   text carries a load, D-18-39);
//! - [`ship`] — `+ship`, `+ship/hold`, `+ship/board` and `+ship/leave`
//!   (D-18-34), and the hull rooms' status line and gauges on `look`.
//!
//! **Doc 18 cut IC8** — finite stock and port commerce:
//!
//! - [`vendor`] — the `PortApplier`'s parts d–e helpers: the authored storefront
//!   and signatory resolved by name in their room at first apply, the clerk, the
//!   storefront vaults, the market vendors, the clerk's grants, the stock plan rows,
//!   the boards and the standing bids;
//! - [`stock`] — `tacsim_port_stock` (mint + receipt), the stock arm and the
//!   lapsed-hold release arm the port tick drives;
//! - [`market`] — the TacSim implementations of the market's seams: the presence
//!   gate, the hull capacity provider and the port institutions' party caps;
//! - [`board`] — the boards 18a names, room by room: what is left, SOLD OUT and
//!   "not buying".

pub mod apply;
pub mod board;
pub mod capacity;
pub mod coop;
pub mod credit;
pub mod def;
pub mod families;
pub mod gauges;
pub mod hull;
pub mod map;
pub mod market;
pub mod readiness;
pub mod ship;
pub mod stock;
pub mod vendor;

pub use coop::{
    Charter, Enrolment, EnrolmentState, EnrolmentStatus, ProgrammePort, live_charter,
    programme_port,
};
pub use def::{
    AskDef, BidDef, MAX_TRANCHE_MAJOR, PORT_MINT_CAP_MAJOR, PORT_ROOM_ROLES, PortDef, PortOrgDef,
    ProgrammeDef, StockRowDef, VendorDef, check_ports, room_key, tranches,
};
pub use families::{
    LegPlan, Manifest, PassagePlan, PortFamilies, PortHandles, StockIssueId, StockReceipt, Ticket,
    TicketId,
};
pub use map::{
    BidRow, ClerkRow, HullRow, PORT_MAP_PREFIX, PortMap, PortMapState, PortOrgRow, StockRow,
    TrancheRow, VendorRow, port_map_key,
};
pub use market::{
    HOLD_ELSEWHERE, HOLD_NOT_HERE, HOLD_UNDER_WAY, HoldPlan, NOT_IN_PORT, NOT_YOUR_CHARTER,
    PlanCompartment, PortCapacity, PortPartyCaps, PortVenueGate, capacity_line, governed_hold,
};
pub use readiness::{Binding, LineState, Readiness, ReadinessLine, readiness, readiness_in};
pub use stock::{
    DISSOLVE_KEY_PREFIX, MAX_CLERK_ROWS, STOCK_ACTION, STOCK_KEY_PREFIX, STOCK_OUTCOME,
    STOCK_SOURCE_KIND,
};
