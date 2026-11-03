//! **The power plant, authored** (doc 39 cut IN.1; Q-39-35).
//!
//! The tenth definition kind and the eighth a pack may carry: a plant is the hardware row a
//! `power` component names, as a mount names its weapon row and a sight its sensor row, so a
//! plant stripped from one hull is the same part as the plant of every other hull that names
//! the row (Q-39-3). It carries its mass and the state set every power role that names it declares; the state a
//! fresh hull's plant starts in stays the role's (a capital hull's starts `online`).
//!
//! A chassis fits one through its `power` component's `plant` field
//! ([`crate::definitions::unit::ComponentRole::Power`]); a power component that names none is
//! refused at load, and one whose states are not the plant's is refused at the catalogue build.
//! `bind` reads nothing from it: the bound capability is the role's states, as before.

use mush_core::CanonicalId;
use serde::{Deserialize, Serialize};

use crate::definitions::DefinitionError;

/// The heaviest plant a row may declare, kilograms (a hauler's is 30,000).
pub const MAX_PLANT_KG: u64 = 1_000_000;

/// **One plant** (doc 39 cut IN.1, Q-39-35): what content says about a power plant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlantDef {
    /// The human-facing name (`vehicle power plant`).
    pub label: String,
    /// What the plant weighs, kilograms: the part's mass (`goods.json` carries the same figure
    /// for every priced plant).
    pub mass_kg: u64,
    /// Its states, in order: `off`, `starting`, `online`.
    pub states: Vec<CanonicalId>,
    /// Why this plant.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl PlantDef {
    /// Re-run every bound.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry: the label's and the note's bounds, a mass
    /// of `0` or past [`MAX_PLANT_KG`], no states, or a repeated state.
    pub fn check(&self, entry: &str) -> Result<(), DefinitionError> {
        crate::definitions::check_label(entry, &self.label)?;
        crate::definitions::check_note(entry, &self.note)?;
        if self.mass_kg == 0 || self.mass_kg > MAX_PLANT_KG {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "a plant weighs 1 to {MAX_PLANT_KG} kg; {} is not",
                    self.mass_kg
                ),
            ));
        }
        if self.states.is_empty() {
            return Err(DefinitionError::value(entry, "a plant names its states"));
        }
        for (i, state) in self.states.iter().enumerate() {
            if self.states[..i].contains(state) {
                return Err(DefinitionError::value(
                    entry,
                    format!("state `{state}` is named twice"),
                ));
            }
        }
        Ok(())
    }
}
