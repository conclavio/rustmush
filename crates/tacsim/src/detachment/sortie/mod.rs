//! Durable physical launches. Ordinary placement remains owned by Sim.
mod commander;
mod commands;
mod facts;
mod field;
mod field_commands;
mod field_orders;
mod model;
pub(crate) mod npc;
mod prepare;
mod runtime;
mod seating;
pub(crate) mod surface;
mod tick;
pub(crate) mod work;

pub(crate) use commands::{
    SWITCHES, command, field_carrier, field_flight, field_ramp, handles, id as operation_id,
    recover_after_exit,
};
pub use field::mission_binding;
pub(crate) use field_commands::{field_order, field_response};
pub use field_orders::{
    Concentration, EngageAt, FieldObjective, FieldOrders, FieldPayload, GIVEN_KEPT, Given, Lapse,
    concentrates,
};
pub use model::{Flight, FlightClose, FlightKey, OperationId, OperationRecord, Phase, UnitHead};
pub use runtime::SortieHandles;
pub use runtime::transport_action_ref;
pub(crate) use runtime::{
    ACTION, action_descriptor, bridge_descriptor, outcome_descriptor, register,
};
pub(crate) use runtime::{availability_key, ctx_available, offer_available, require_available};
pub use seating::NPC_SEAT_NO_FIRE;
pub use surface::SurfaceCrew;
pub use work::{FieldStep, FieldWork};

/// **The focus-fire order's usage** (doc 44 unit MW.14c).
pub const CONCENTRATE_USAGE: &str =
    "Use <slot|ALL> CONCENTRATE <contact>: one contact of your own scope, by its designator.";
/// A `CONCENTRATE` naming a contact its leader has not identified (doc 44 unit MW.14c).
pub const CONCENTRATE_UNIDENTIFIED: &str = "CONCENTRATE needs a contact you have identified: a provisional number is yours alone, and no other unit could find it.";
/// A `CONCENTRATE` naming nothing in its leader's own scope, or given off a running field (doc
/// 44 unit MW.14c).
pub const CONCENTRATE_NO_CONTACT: &str =
    "No such contact in your own scope. CONCENTRATE names a contact inside a running field.";
/// A `CONCENTRATE` given an `EXPIRES` (doc 44 unit MW.14c).
pub const CONCENTRATE_NO_EXPIRY: &str = "CONCENTRATE takes no EXPIRES: it ends when its contact is destroyed or lost, or when you order otherwise.";
/// **`+sortie/orders`' head line over an NPC leader's orders** (doc 44 unit MW.14c).
pub const NPC_ORDERS_HEAD: &str =
    "Orders your NPC leader gave, oldest first (a reply shows once it has reached the leader):";
/// **One order an NPC leader gave, in `+sortie/orders`** (doc 44 unit MW.14c): `{leader}` the
/// NPC's name, `{slot}`, `{sequence}`, `{order}` as a leader types it, `{status}` `no reply
/// confirmed` or the reply's word upper-cased and `received`.
pub const NPC_ORDER: &str = "{leader} to slot {slot}, order {sequence}: {order}; {status}.";

/// **`+sortie/orders`' head line** (`{term}` the command term; `{roster}`, `{revision}` the
/// fence's other two numbers): the prefill a leader copies to issue an order. A player client
/// reads it (the bot's captain, doc 32 G-4), so it is a constant.
pub const ORDER_PREFILL: &str =
    "Field command term {term}. Order prefill: ORDER DT:{roster}:{revision}:{term} ALL HOLD";
/// **One sent order in `+sortie/orders`** (`{status}` `submitted; no reply confirmed`, or the
/// reply's word upper-cased and `received`: `WILCO received`, `NEGATIVE received`).
pub const ORDER_SENT: &str = "Sent slot {slot} order {sequence}: {status}. Execution unconfirmed.";
/// The leader who gave an order, when it lapses with their absence (doc 45 unit AP.8;
/// Q-45-13): `{order}` as they typed it, `{slot}`. Queued for one who is not on watch.
pub const ORDER_LAPSED: &str = "While you were away your order {order} to slot {slot} lapsed. The unit stopped where it was and holds its fire; if it is hit it answers from where it stands. The order stays lapsed until you give it again.";
/// The roster's leader now, when the order of the leader before them lapses: `{order}`,
/// `{slot}`.
pub const ORDER_LAPSED_LEAD: &str = "Slot {slot}'s order {order} lapsed with the leader who gave it. The unit stands where it is, and answers only if it is hit, until you order it.";

/// **What `+sortie/orders` says after a sent order's line once the order lapsed** (doc 45 unit
/// AP.8; Q-45-13): an `ENGAGE` or an `ADVANCE` whose leader went away. A sentence of its own,
/// after [`ORDER_SENT`]'s, so the line a player client parses for the reply's word is unchanged
/// (`mush-bot`'s `order_replies` pins it).
pub const ORDER_LAPSED_STATUS: &str =
    "It lapsed while you were away; give it again if you want it.";
