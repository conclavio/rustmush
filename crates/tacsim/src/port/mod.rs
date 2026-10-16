//! **The trade run's port code** (doc 18 cut IC2; doc 18 §4.2 "Compartments",
//! §4.3 "The capacity function", §4.4).
//!
//! [`capacity`] is the one pure function a hull's stowage compartments are
//! checked through, everywhere a load is asked about: the market lot leg's
//! pre-check and the Draw prepare (cuts IC7/IC8), `+ship/plan` and
//! `+ship/hold`, the room gauges (IC6) and the queue's buy clamp and `fill`
//! (IC11). The port tick (cut IC5, `port_tick.rs`) and the passage planner
//! (cut IC9, `passage.rs`) are separate files beside it (doc 18 §5).

pub mod capacity;
