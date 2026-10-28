//! Neutral detachment command contracts (doc 26 DT.1).
//!
//! These records neither reserve units nor convey station, asset, or communication
//! authority. Hosts resolve those facts before admission. An OOC adapter may use
//! the same contract without making the simulator's lobby rules into IC policy.

pub mod orders;
pub mod roster;

pub use orders::*;
pub use roster::*;
