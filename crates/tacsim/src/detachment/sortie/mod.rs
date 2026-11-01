//! Durable physical launches. Ordinary placement remains owned by Sim.
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
    SWITCHES, command, field_ramp, handles, id as operation_id, recover_after_exit,
};
pub use field::mission_binding;
pub(crate) use field_commands::{field_order, field_response};
pub use field_orders::{EngageAt, FieldObjective, FieldOrders, FieldPayload};
pub use model::{Flight, FlightClose, FlightKey, OperationId, OperationRecord, Phase, UnitHead};
pub use runtime::SortieHandles;
pub use runtime::transport_action_ref;
pub(crate) use runtime::{
    ACTION, action_descriptor, bridge_descriptor, outcome_descriptor, register,
};
pub(crate) use runtime::{availability_key, ctx_available, offer_available, require_available};
pub use surface::SurfaceCrew;
pub use work::{FieldStep, FieldWork};

/// **`+sortie/orders`' head line** (`{term}` the command term; `{roster}`, `{revision}` the
/// fence's other two numbers): the prefill a leader copies to issue an order. A player client
/// reads it (the bot's captain, doc 32 G-4), so it is a constant.
pub const ORDER_PREFILL: &str =
    "Field command term {term}. Order prefill: ORDER DT:{roster}:{revision}:{term} ALL HOLD";
/// **One sent order in `+sortie/orders`** (`{status}` `submitted; no reply confirmed`, or the
/// reply's word upper-cased and `received`: `WILCO received`, `NEGATIVE received`).
pub const ORDER_SENT: &str = "Sent slot {slot} order {sequence}: {status}. Execution unconfirmed.";
