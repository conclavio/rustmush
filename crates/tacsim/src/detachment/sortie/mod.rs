//! Durable physical launches. Ordinary placement remains owned by Sim.
mod commands;
mod facts;
mod model;
mod prepare;
mod runtime;
mod seating;
mod tick;

pub(crate) use commands::{SWITCHES, command, handles};
pub use model::{Flight, FlightClose, FlightKey, OperationId, OperationRecord, Phase, UnitHead};
pub use runtime::SortieHandles;
pub(crate) use runtime::{
    ACTION, action_descriptor, bridge_descriptor, outcome_descriptor, register,
};
pub(crate) use runtime::{availability_key, offer_available, require_available};
