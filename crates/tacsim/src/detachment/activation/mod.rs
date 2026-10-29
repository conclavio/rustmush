//! Accepted rosters and exclusive reservations. No movement or launch authority.
mod commands;
mod model;
mod preflight;
mod runtime;

pub(crate) use commands::{SWITCHES, command, handles};
pub use model::{ActiveDetachment, Reservation, detachment_id};
pub use runtime::{ActivationHandles, action_descriptor, outcome_descriptor, register};
