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

pub(crate) use commands::{SWITCHES, command, handles, id as operation_id, recover_after_exit};
pub use field::mission_binding;
pub(crate) use field_commands::{field_order, field_response};
pub use field_orders::{FieldObjective, FieldOrders, FieldPayload};
pub use model::{Flight, FlightClose, FlightKey, OperationId, OperationRecord, Phase, UnitHead};
pub use runtime::SortieHandles;
pub use runtime::transport_action_ref;
pub(crate) use runtime::{
    ACTION, action_descriptor, bridge_descriptor, outcome_descriptor, register,
};
pub(crate) use runtime::{availability_key, offer_available, require_available};
pub use surface::SurfaceCrew;
pub use work::{FieldStep, FieldWork};
