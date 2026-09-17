//! **What a unit sees with** (16b cut C.2, C1; doc 16 §3.6 "The pass", §4
//! "Content packs").
//!
//! The row cut S's perception pass reads: the candidate radius in cells, the
//! attenuation a `Partial` line of sight may carry and still count, how long a
//! contact must be held before it classifies, whether wrecks are candidates at
//! all — and the **two null hooks**, `emission` and `jamming`, which doc 16 §3.6
//! requires to be *present* and *null* today so that the ECM and ECCM extensions
//! of §9 are a value change rather than a schema change.
//!
//! "Present and null" is load-bearing and is the reason both are
//! `Option<Value>` without a serde default: a row that simply omits them is
//! refused by the decoder for a missing field, and a row that fills one is
//! refused by [`SensorDef::check`] naming the hook and the cut that owns it.

use serde::{Deserialize, Serialize};

use crate::definitions::DefinitionError;

/// The widest candidate radius one sensor may declare, in cells. The arena is
/// 256 cells a side (D-16-5), so this is four arenas.
pub const MAX_RANGE_CELLS: u16 = 1_024;

/// The longest a classification may take: ten minutes is past any engagement.
pub const MAX_CLASSIFY_AFTER_MS: u32 = 600_000;

/// One sensor (doc 16 §3.6).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorDef {
    /// The human-facing name.
    pub label: String,
    /// The candidate radius at signature factor 1, in cells (doc 16 §3.6 step
    /// 1; 16b §9 pins 60 — 600 m at `cell_cm` 1,000).
    pub max_range_cells: u16,
    /// The most `Opacity::Partial` attenuation a line of sight may carry and
    /// still be a detection (doc 16 §3.6 step 2). The atlas prices light woods
    /// at 25 and heavy woods at 60.
    pub max_attenuation: u8,
    /// How long a contact must be held, without a gap longer than
    /// `sensors.pass_ms`, before it classifies (doc 16 §3.6 step 4).
    pub classify_after_ms: u32,
    /// Whether a wreck is a candidate (doc 16 §3.6 step 1).
    pub sees_wrecks: bool,
    /// **The ECM hook** (doc 16 §3.6 last line, §9). Present and null.
    pub emission: Option<serde_json::Value>,
    /// **The ECCM hook** (doc 16 §3.6 last line, §9). Present and null.
    pub jamming: Option<serde_json::Value>,
    /// Why these numbers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl SensorDef {
    /// Re-run every bound, including the two hooks' nullity.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry and the field.
    pub fn check(&self, entry: &str) -> Result<(), DefinitionError> {
        crate::definitions::check_label(entry, &self.label)?;
        crate::definitions::check_note(entry, &self.note)?;
        if self.max_range_cells == 0 || self.max_range_cells > MAX_RANGE_CELLS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "max_range_cells is 1 to {MAX_RANGE_CELLS}; {} is not",
                    self.max_range_cells
                ),
            ));
        }
        if self.max_attenuation > 100 {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "max_attenuation is 0 to 100 (the atlas's own scale); {} is not",
                    self.max_attenuation
                ),
            ));
        }
        if self.classify_after_ms > MAX_CLASSIFY_AFTER_MS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "classify_after_ms is at most {MAX_CLASSIFY_AFTER_MS}; {} is not",
                    self.classify_after_ms
                ),
            ));
        }
        for (hook, value, owner) in [
            ("emission", self.emission.as_ref(), "ECM"),
            ("jamming", self.jamming.as_ref(), "ECCM"),
        ] {
            if value.is_some_and(|value| !value.is_null()) {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "`{hook}` is the {owner} hook of doc 16 §9 and is null until the cut \
                         that reads it lands; a value here configures nothing"
                    ),
                ));
            }
        }
        Ok(())
    }
}
