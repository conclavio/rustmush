//! Published governed hardware content and the explicit production adapters.
//! A loaded recipe is data; only its actual acknowledged publication can be
//! used by Industry. Generic commodity recipes never consume these entries.

mod composition;
mod content;
mod recipes;
pub(crate) use composition::Registration;
pub(crate) use composition::RosterPolicy;
pub use composition::{GovernedHardware, HardwareDeployment, HardwareOwners, UngoverningSite};
pub use content::{
    CATALOGUE, HardwareCatalogueDef, HardwareProcess, HardwareRecipeDef, PACK, VERSION,
};
pub use recipes::PublishedHardware;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod flow;
