//! **What a weapon fires** (16b cut C.2, C1; doc 16 §3.5 "Ammunition
//! definition", §4 "Content packs").
//!
//! Three numbers and a carried word: `damage` (one fixed value — the stock
//! rules roll to hit and never to wound), `penetration_class` (carried, unused
//! by the stock rules, so the laser and the sabot of §9 have somewhere to put
//! themselves), and `count_per_load`, which is the magazine a `line_tank`'s
//! component cell is sized from.
//!
//! Integers only (D-16-4), and every bound is re-checked on load rather than at
//! the first shot.

use mush_core::CanonicalId;
use serde::{Deserialize, Serialize};

use crate::definitions::DefinitionError;

/// The most damage one round may carry.
///
/// A magnitude bound beside the shape bounds: cut W's overflow arithmetic adds
/// a round's damage to a facet's armour and then to a section's internals, and
/// a `u32::MAX` round would make that sum a function of authored data rather
/// than of the model. A hundred thousand is three orders of magnitude past the
/// stock cannon.
pub const MAX_DAMAGE: u32 = 100_000;

/// The most rounds one load may hold.
pub const MAX_COUNT_PER_LOAD: u32 = 100_000;

/// One ammunition family (doc 16 §3.5).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AmmunitionDef {
    /// The human-facing name.
    pub label: String,
    /// One fixed damage value. There is no damage roll in the stock rules.
    pub damage: u32,
    /// Carried, and read by nothing in the stock rules (doc 16 §3.5).
    pub penetration_class: CanonicalId,
    /// How many rounds one load holds — the size of the firing component's
    /// `ammo` cell.
    pub count_per_load: u32,
    /// Why these numbers. Authored prose; skipped when empty, so an entry that
    /// says nothing costs nothing in the payload digest.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl AmmunitionDef {
    /// Re-run every bound.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry and the field.
    pub fn check(&self, entry: &str) -> Result<(), DefinitionError> {
        crate::definitions::check_label(entry, &self.label)?;
        crate::definitions::check_note(entry, &self.note)?;
        if self.damage == 0 || self.damage > MAX_DAMAGE {
            return Err(DefinitionError::value(
                entry,
                format!("damage is 1 to {MAX_DAMAGE}; {} is not", self.damage),
            ));
        }
        if self.count_per_load == 0 || self.count_per_load > MAX_COUNT_PER_LOAD {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "count_per_load is 1 to {MAX_COUNT_PER_LOAD}; {} is not",
                    self.count_per_load
                ),
            ));
        }
        Ok(())
    }
}
