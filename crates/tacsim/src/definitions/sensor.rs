//! **What a unit sees with** (16b cut C.2, C1; doc 16 §3.6 "The pass", §4
//! "Content packs").
//!
//! The row cut S's perception pass reads: the candidate radius in cells, the
//! attenuation a `Partial` line of sight may carry and still count, how long a
//! contact must be held before it classifies, whether wrecks are candidates at
//! all, **the signature table** doc 16 §3.6 step 3 scales the range by, **the
//! comms reach** step 5 shares along — and the **two null hooks**, `emission`
//! and `jamming`, which doc 16 §3.6 requires to be *present* and *null* today so
//! that the ECM and ECCM extensions of §9 are a value change rather than a schema
//! change.
//!
//! "Present and null" is load-bearing and is the reason both are
//! `Option<Value>` without a serde default: a row that simply omits them is
//! refused by the decoder for a missing field, and a row that fills one is
//! refused by [`SensorDef::check`] naming the hook and the cut that owns it.

use mush_atlas::TerrainClass;
use serde::{Deserialize, Serialize};

use crate::definitions::DefinitionError;

/// The widest candidate radius one sensor may declare, in cells. The arena is
/// 256 cells a side (D-16-5), so this is four arenas.
pub const MAX_RANGE_CELLS: u16 = 1_024;

/// The longest a classification may take: ten minutes is past any engagement.
pub const MAX_CLASSIFY_AFTER_MS: u32 = 600_000;

/// The widest comms reach one sensor may declare, in cells (16b §9 pins 30).
/// Bounded by the arena's own side, because a share beyond the window is a share
/// to nobody.
pub const MAX_SHARE_RANGE_CELLS: u16 = 256;

/// A signature factor is per mille, and `1000` — the factor of a target that
/// hides nothing — is its ceiling. A row above it would *lengthen* a sensor's
/// range for a particular target, which is the `max_signature_factor` knob's job
/// and not content's (doc 16 §3.6 steps 1 and 3).
pub const MAX_SIGNATURE_PERMILLE: u32 = 1_000;

/// **One row of the signature table** (16b cut S.1; doc 16 §3.6 step 3: *a
/// stopped tank in heavy woods has a lower factor than one moving on a road*).
///
/// The factor scales the sensor's range for a target standing on `class`, per
/// mille: `1000` is a target that hides nothing, `500` a target detected at half
/// the sensor's reach. `moving` is read when the target's speed is non-zero and
/// `stopped` otherwise, which is the whole of the *posture* doc 16 §3.6 names —
/// there is no third state, and the arena reads the speed it already stores
/// rather than a derived one.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignatureRow {
    /// The ground the target is standing on.
    pub class: TerrainClass,
    /// The factor while the target is under way, per mille.
    pub moving_permille: u32,
    /// The factor while the target is stopped, per mille.
    pub stopped_permille: u32,
}

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
    /// **The signature table** (doc 16 §3.6 step 3), by the ground the target
    /// stands on. A class the table does not name is `1000`/`1000` — a target
    /// that hides nothing — so the table is the list of grounds that *conceal*
    /// and never a list every class has to appear in.
    pub signature: Vec<SignatureRow>,
    /// **The comms reach** a classified contact is shared along (doc 16 §3.6
    /// step 5, W6), in cells; 16b §9 pins 30. Zero shares nothing.
    pub share_range_cells: u16,
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
        if self.share_range_cells > MAX_SHARE_RANGE_CELLS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "share_range_cells is 0 to {MAX_SHARE_RANGE_CELLS}; {} is not",
                    self.share_range_cells
                ),
            ));
        }
        let mut seen: Vec<TerrainClass> = Vec::with_capacity(self.signature.len());
        for row in &self.signature {
            if seen.contains(&row.class) {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "the signature table names `{}` twice; one ground is one row",
                        row.class.label()
                    ),
                ));
            }
            seen.push(row.class);
            for (posture, permille) in [
                ("moving_permille", row.moving_permille),
                ("stopped_permille", row.stopped_permille),
            ] {
                if permille > MAX_SIGNATURE_PERMILLE {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "the signature table's `{}` row has {posture} {permille}, past \
                             {MAX_SIGNATURE_PERMILLE}: a factor lengthens no sensor's range, it \
                             only shortens it",
                            row.class.label()
                        ),
                    ));
                }
            }
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
