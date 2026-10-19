//! Published governed hardware content and the explicit production adapters.
//! A loaded recipe is data; only its actual acknowledged publication can be
//! used by Industry. Generic commodity recipes never consume these entries.

mod content;
mod recipes;
pub use content::{
    CATALOGUE, HardwareCatalogueDef, HardwareProcess, HardwareRecipeDef, PACK, VERSION,
};
pub use recipes::PublishedHardware;

#[cfg(test)]
mod tests;
