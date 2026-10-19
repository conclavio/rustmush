//! **The trade run's port code** (doc 18 cut IC2; doc 18 §4.2 "Compartments",
//! §4.3 "The capacity function", §4.4, §4.7).
//!
//! [`crate::port::capacity`] is the one pure function a hull's stowage compartments are
//! checked through, everywhere a load is asked about: the market lot leg's
//! pre-check and the Draw prepare (cuts IC7/IC8), `+ship/plan` and
//! `+ship/hold`, the room gauges (IC6) and the queue's buy clamp and `fill`
//! (IC11). The port tick (cut IC5, [`crate::port_tick`], at the crate root per
//! doc 18 §5's lane note) and the passage planner (cut IC9, `passage.rs`) are
//! separate files (doc 18 §5).
//!
//! **Doc 18 cut IC3** binds the corridor's ports into the world:
//!
//! - [`crate::port::def`] — the `port` pack entry ([`crate::port::PortDef`]) and its checks;
//! - [`crate::port::map`] — the applier's fail-closed record of what it made, one
//!   `tacsim:port:{pack}/{entry}` document per port entry;
//! - [`crate::port::apply`] — the `PortApplier`: the domain, the treasury, the orgs and
//!   their capital (doc 18 §4.7 parts a–c), once, under the content latch;
//! - [`mod@crate::port::readiness`] — the readiness report `@tacsim/port status` renders and
//!   cut IC6 gates `+coop/enrol` on;
//! - [`crate::port::credit`] — `+credit`, the pilot's own account.
//!
//! **Doc 18 cut IC4** makes the programme's hulls places people stand in:
//!
//! - [`crate::port::hull`] — `PortApplier` part (f): each hull Thing, its unit, its seven
//!   rooms and five exit pairs from the Dray's `interior` template, the store
//!   and airlock spaces with the airlock's member rooms and berths, the laid-up
//!   placement and the title under `tacsim.registry`; and the readiness
//!   report's hulls line.
//!
//! Doc 18 cut IC5 — [`crate::port::families`]: the stock-issue receipt and the departure
//! ticket, with the `PassagePlan` they freeze; [`crate::port_tick`] is the tick.
//!
//! **Doc 18 cut IC6** — the Co-op's First Captain programme and boarding:
//!
//! - [`crate::port::coop`] — `+coop`, `+coop/enrol` and `+coop/statement`; the enrolment
//!   document (one per gateway account, ever, D-18-25), the operate gate
//!   ([`crate::port::live_charter`]: the live lease, never the document, D-18-23), the
//!   free-hull rule and the observer that folds a staff-ended lease;
//! - [`crate::port::gauges`] — the pure gauge renders over the capacity function (no pack
//!   text carries a load, D-18-39);
//! - [`crate::port::ship`] — `+ship`, `+ship/hold`, `+ship/board` and `+ship/leave`
//!   (D-18-34), and the hull rooms' status line and gauges on `look`.
//!
//! **Doc 18 cut IC8** — finite stock and port commerce:
//!
//! - [`crate::port::vendor`] — the `PortApplier`'s parts d–e helpers: the authored storefront
//!   and signatory resolved by name in their room at first apply, the clerk, the
//!   storefront vaults, the market vendors, the clerk's grants, the stock plan rows,
//!   the boards and the standing bids;
//! - [`crate::port::stock`] — `tacsim_port_stock` (mint + receipt), the stock arm and the
//!   lapsed-hold release arm the port tick drives;
//! - [`crate::port::market`] — the TacSim implementations of the market's seams: the presence
//!   gate, the hull capacity provider and the port institutions' party caps;
//! - [`crate::port::board`] — the boards 18a names, room by room: what is left, SOLD OUT and
//!   "not buying".
//!
//! **Doc 18 cut IC9** — undock and fly, the first player-runnable flight:
//!
//! - [`crate::port::passage`] — the passage planner over the sky at departure (§4.4, §4.8):
//!   `plan_passage`, `choose_route`, the intercept passes, the Coast burn, the
//!   crew-day rule and the payload;
//! - [`crate::port::burn`] — `tacsim_port_burn`: the burn's prepare recomputes the payload,
//!   the burn and the crew-days from the filed plan and the drive table it holds,
//!   debits the tank and the locker and writes the ticket, matter first, in one
//!   bundle (D-18-11, D-18-46);
//! - [`crate::port::voyage`] — the filed sailing plan (`tacsim:voyage:{hull}`), `+ship/plan`,
//!   `+ship/undock`, the launch arm the port tick drives, the staff terminal for a
//!   held launch and the passage renders.
//!
//! **Doc 18 cut IC10** — selling and the Co-op share:
//!
//! - [`crate::port::proceeds`] — [`crate::port::CoopProceeds`], the market's proceeds policy: the share of the
//!   margin over the programme's live reference ask paid to the Trust out of a sale
//!   from an enrolled hold, the sale line, and the enrolment's sale counters folded
//!   once per settled sale.
//!
//! **Doc 18 cut IC11** — captain's orders, the first cut with the full daily loop:
//!
//! - [`crate::port::when`] — the UTC time helper: `at=`, `until=`, the one rule for when a
//!   depart is due (resolved at its reach) and the OOC departure clock;
//! - [`crate::port::orders`] — the queued sailing plan (`tacsim:orders:{hull}`, D-18-31), its
//!   closed vocabulary, its hold shapes and the verbs `+ship/orders` and
//!   `+ship/order`;
//! - [`crate::port::executor`] — the planning core `+ship/plan` and the order arm share (the
//!   venue's asks and bids, the clamps and their bound words, the projection,
//!   `fill` and the reserve, D-18-41) and the order arm the port tick drives
//!   (D-18-32).
//!
//! **Doc 18 cut IC13** — buyout, graduation and dormancy:
//!
//! - [`buyout`] — `+coop/buyout`: the pilot-issued asset-purchase contract
//!   (`TitleHeld{hull, Trust}` + `Before`, `TitleTransfer` Trust → pilot), the
//!   pilot's escrow, the charter closed with `end_lease` and the Trust's
//!   acceptance last (lead ruling L1); the observer's verdict that graduates on
//!   settlement and re-charters the pilot on a refund (L7);
//! - [`recall`] — the recall in two halves (end the charter; return the hull,
//!   L4): `+coop/release`, staff `@coop/revoke`, and the port tick's recall arm
//!   (dormancy after `dormancy_real_days` without a departure, L3, and the
//!   return of a hull whose charter ended elsewhere); the hold vault is re-homed
//!   to the Trust, never closed (L5).

pub mod apply;
pub mod board;
pub mod burn;
pub mod buyout;
pub mod capacity;
/// Doc 19 cut A.3: `+chart` — the sky from where you stand, the passages now and
/// the window over the next IC year (doc 19 §5).
pub mod chart;
pub mod coop;
pub mod credit;
pub mod def;
pub mod executor;
pub mod families;
pub mod gauges;
pub mod hull;
pub mod map;
pub mod market;
pub mod orders;
pub mod passage;
/// Doc 19 cut A.4: the reach premium on a port's demand-side bids (doc 19 §6).
pub mod price;
pub mod proceeds;
pub mod readiness;
pub mod recall;
pub mod ship;
pub mod stock;
pub mod vendor;
pub mod voyage;
pub mod when;

pub use burn::{BURN_ACTION, BURN_KEY_PREFIX, BURN_OUTCOME};
pub use chart::{
    CHART_HELP, CHART_NO_BODY, CHART_NO_LANE, CHART_NO_ROUTE, CHART_NOWHERE, CHART_VERB,
    PLAN_WINDOW_DAYS, WINDOW_DAYS,
};
pub use coop::{
    Charter, EndCause, Enrolment, EnrolmentState, EnrolmentStatus, Helm, ProgrammePort, Standing,
    charter_tenant, enrol_gate, gated, helm_holder, helm_of, live_charter, programme_port, shuts,
};
pub use def::{
    AskDef, BidDef, EmergencyDef, MAX_EMERGENCY_GOODS, MAX_EMERGENCY_RATIO_PERMILLE,
    MAX_PREMIUM_PERMILLE, MAX_TRANCHE_MAJOR, PORT_MINT_CAP_MAJOR, PORT_ROOM_ROLES, PortDef,
    PortOrgDef, ProgrammeDef, ReachPremiumDef, StockRowDef, VendorDef, check_ports, room_key,
    tranches,
};
pub use executor::{
    Ask, Bid, Bound, FILL_ITERATIONS, FillError, FillTargets, MAX_ORDER_ATTEMPTS,
    MAX_ORDER_REFUSALS, MAX_ORDER_VERIFY_PER_FIRE, ORDER_RESTAGE_SECS, ORDER_RETRY_SECS,
    ORDER_SETTLE_GRACE_SECS, ask_at, bid_at, clamp_buy, clamp_sell, fill_targets, funds_units,
    join_bounds, project_cargo, reserve_for, room_for,
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
    HOLD_CLEARED, HOLD_ELSEWHERE, HOLD_NOT_HERE, HOLD_PAPERS_DARK, HOLD_UNDER_WAY, HoldPlan,
    NOT_IN_PORT, NOT_YOUR_CHARTER, PlanCompartment, PortCapacity, PortPartyCaps, PortVenueGate,
    capacity_line, governed_hold,
};
pub use orders::{
    HoldNote, Inflight, InflightRefs, LogLine, MAX_DEPART_CREW, MAX_ORDER_LOG_BYTES,
    MAX_ORDER_LOG_LINES, MAX_ORDERS, MAX_PRICE_MAJOR, ORDERS_PREFIX, ORDERS_SCHEMA,
    ORDERS_SHOWN_LOG, OnShort, Order, OrderState, Orders, OrdersState, PlanState, Qty, Reached,
    What, orders_key, orders_key_owner, orders_state,
};
pub use passage::{
    DAY_SECS, DEFAULT_THROTTLE_PERMILLE, DriveLaw, FlipProfile, G0_MPS2, HullDrive,
    INTERCEPT_PASSES, KEEP_OUT_AU, MAX_THROTTLE_PERMILLE, MIN_THROTTLE_PERMILLE, PlanError,
    RoutePolicy, choose_route, crew_days, detour_km, flip_accel_mps2, flip_profile,
    intercept_passes, leg_distance_km, leg_seconds, passage_burn, payload_kg, plan_passage,
};
pub use price::{
    DAY_S, EMERGENCY_CHECK_SECS, EmergencyReading, MAX_SCAN_DAYS, MAX_SCAN_SAMPLES, Reach,
    bid_price_major, emergency_crossed, emergency_readings, emergency_reference_major, median_of,
    premium_permille, priced, reach_for, reach_ratio_permille, scan_days, synodic_days,
};
pub use proceeds::{
    BASELINE_MEMO, CoopProceeds, REFERENCE_DARK, SALE_BOOKS_DARK, SALE_LINE, SALE_LINE_NO_SHARE,
    SALE_NOT_PILOT, SHARE_LABEL, share_of,
};
pub use readiness::{Binding, LineState, Readiness, ReadinessLine, readiness, readiness_in};
pub use stock::{
    DISSOLVE_KEY_PREFIX, MAX_CLERK_ROWS, STOCK_ACTION, STOCK_KEY_PREFIX, STOCK_OUTCOME,
    STOCK_SOURCE_KIND,
};
pub use voyage::{
    LAUNCH_GRACE_FIRES, LAUNCH_WATCH_FIRES, POLICY_USAGE, PassageOptions, PendingVoyage,
    THROTTLE_USAGE, VIA_UNKNOWN, VOYAGE_PREFIX, VOYAGE_SCHEMA, Voyage, VoyageState, voyage_key,
    voyage_key_owner,
};
pub use when::{MAX_AHEAD_SECS, MAX_WINDOW_SECS, NotBefore};
