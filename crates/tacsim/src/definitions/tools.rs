//! **The terrain vehicle's instruments, as data** (doc 20 cut SM.3a; doc 20
//! §4.1 "Tools are a new content kind" and "Charge").
//!
//! Three compiled-in documents under `crates/tacsim/content/tools/` — the Mk1
//! core drill, the Mk1 survey mast and the Mk1 rig battery — each one strict
//! typed [`ToolDef`] per file (`deny_unknown_fields`, integers only), checked on
//! load and pinned by its canonical digest under [`TOOLS_DIGEST_DOMAIN`]
//! (`crates/tacsim/tests/content.rs`).
//!
//! # Why a typed document and not a pack
//!
//! The goods table's precedent (lead ruling R12, [`super::goods`]): nothing
//! binds a tool in phase 1. A tool mints no `DefinitionRef` and fits no unit
//! until the promotion cut (doc 20 §17) lowers it to `ComponentRole::Tool`; a
//! pack kind now would mint definitions and descriptors nobody reads. Until then
//! the rig pool's `vehicles.tools` row (lane v10) names the three instruments
//! every rig carries, and the cuts that drive, scan, sound and drill (SM.3b,
//! SM.4, SM.7) read their numbers from [`tools_table`].
//!
//! Every radius is **per axis** — a square of `2r + 1` cells a side, the site
//! grid's own metric — and every charge figure is in charge units (`cu`; one cu
//! is four watt-hours, the battery's note states the arithmetic). The battery is
//! the cut's addition to doc 20 §4.1's two instruments: §4.1's charge figures, as
//! data only.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, DefinitionDigest};
use serde::{Deserialize, Serialize};

use super::{DefinitionError, check_label, check_note};

/// The domain each tool's canonical digest is taken under; the encoding
/// version rides it, as a pack digest's does.
pub const TOOLS_DIGEST_DOMAIN: &str = "tacsim.tools.v1";

/// The largest charge figure any tool may author, charge units.
pub const MAX_TOOL_CU: u32 = 1_000_000;

/// The longest duration any tool may author, seconds (one day).
pub const MAX_TOOL_SECONDS: u32 = 86_400;

/// The hardest rock a drill may be rated for (a locomotion-table factor).
pub const MAX_ROCK_FACTOR: u32 = 10;

/// The most whole tonnes one drill cycle may yield.
pub const MAX_YIELD_T: u32 = 24;

/// The widest radius, cells per axis: half a site's 256-cell side.
pub const MAX_RADIUS_CELLS: u32 = 128;

/// The tallest mast, centimetres (100 m).
pub const MAX_MAST_CM: u32 = 10_000;

/// **One tool** (doc 20 §4.1).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDef {
    /// The document's version. A changed figure is a new version.
    pub version: u32,
    /// The tool's id (`tacsim.tool.drill_core_mk1`).
    pub id: CanonicalId,
    /// The human-facing name (`Mk1 core drill`).
    pub label: String,
    /// What the tool is, and its figures.
    pub role: ToolRole,
    /// Why these numbers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// **What a tool is** — externally tagged on the wire, `{"drill": {…}}`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolRole {
    /// The core drill (doc 20 §4.1 "Drill").
    Drill(DrillDef),
    /// The survey mast (doc 20 §4.1 "Scanner").
    Scanner(ScannerDef),
    /// The rig battery (doc 20 §4.1 "Charge").
    Battery(BatteryDef),
}

/// **A drill's figures** (doc 20 §4.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrillDef {
    /// One cycle, seconds; `1..=86_400`.
    pub cycle_s: u32,
    /// The hardest rock factor the bit cuts; `1..=10`.
    pub rock_factor: u32,
    /// The charge one cycle draws, cu; `1..=MAX_TOOL_CU`.
    pub energy_cu: u32,
    /// Whole tonnes one cycle yields by richness.
    pub yield_t: YieldDef,
}

/// **A cycle's yield by richness**, whole tonnes: `1 ≤ lean ≤ fair ≤ rich ≤ 24`
/// (a barren block yields nothing and is not a row; "never rising" with depth is
/// a pocket rule, not this one).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct YieldDef {
    /// From a lean block.
    pub lean: u32,
    /// From a fair block.
    pub fair: u32,
    /// From a rich block.
    pub rich: u32,
}

/// **A survey mast's figures** (doc 20 §4.1): the wide passive scan and the
/// narrow active sounding, each with its time and its charge.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScannerDef {
    /// One scan, seconds; `1..=86_400`.
    pub scan_s: u32,
    /// The scan's reach, cells per axis.
    pub scan_radius_cells: u32,
    /// The mast's height, centimetres; `1..=10_000`.
    pub mast_cm: u32,
    /// One scan's charge, cu.
    pub scan_cu: u32,
    /// One sounding, seconds; `1..=86_400`.
    pub sound_s: u32,
    /// The sounding's reach, cells per axis.
    pub sound_radius_cells: u32,
    /// The ring the sounding reads finest, cells per axis.
    pub quiet_radius_cells: u32,
    /// One sounding's charge, cu.
    pub sound_cu: u32,
}

/// **A rig battery's figures** (doc 20 §4.1 "Charge").
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatteryDef {
    /// A full battery, cu.
    pub capacity_cu: u32,
    /// What the bay's umbilical puts back in an IC hour, cu.
    pub bay_recharge_cu_per_ic_hour: u32,
    /// What driving one cell of nominal locomotion cost (100) draws, cu.
    pub drive_cu_per_nominal_cell: u32,
    /// The charge a rig keeps back to drive home, permille of capacity;
    /// `1..=999`.
    pub reserve_permille: u32,
}

/// A tool-level refusal under the document's name.
fn refuse(name: &str, detail: impl Into<String>) -> DefinitionError {
    DefinitionError::value(name, detail)
}

/// A figure inside `lo..=hi`, or the sentence naming it.
fn within(name: &str, field: &str, value: u32, lo: u32, hi: u32) -> Result<(), DefinitionError> {
    if value < lo || value > hi {
        return Err(refuse(
            name,
            format!("{field} is {lo} to {hi}; {value} is not"),
        ));
    }
    Ok(())
}

impl ToolDef {
    /// **Decode a tool and re-run every bound** (doc 20 §4.1).
    ///
    /// # Errors
    ///
    /// [`DefinitionError`] under `name` (the file) naming the field and the
    /// value.
    pub fn load(name: &str, bytes: &str) -> Result<ToolDef, DefinitionError> {
        let tool: ToolDef =
            serde_json::from_str(bytes).map_err(|error| refuse(name, error.to_string()))?;
        tool.check(name)?;
        Ok(tool)
    }

    /// Re-run every bound.
    ///
    /// # Errors
    ///
    /// As [`ToolDef::load`].
    pub fn check(&self, name: &str) -> Result<(), DefinitionError> {
        if self.version == 0 {
            return Err(refuse(name, "version 0 is reserved"));
        }
        check_label(name, &self.label)?;
        check_note(name, &self.note)?;
        match &self.role {
            ToolRole::Drill(drill) => {
                within(name, "drill.cycle_s", drill.cycle_s, 1, MAX_TOOL_SECONDS)?;
                within(
                    name,
                    "drill.rock_factor",
                    drill.rock_factor,
                    1,
                    MAX_ROCK_FACTOR,
                )?;
                within(name, "drill.energy_cu", drill.energy_cu, 1, MAX_TOOL_CU)?;
                let YieldDef { lean, fair, rich } = drill.yield_t;
                if !(1 <= lean && lean <= fair && fair <= rich && rich <= MAX_YIELD_T) {
                    return Err(refuse(
                        name,
                        format!(
                            "drill.yield_t is 1 <= lean <= fair <= rich <= {MAX_YIELD_T} whole \
                             tonnes; lean {lean}, fair {fair}, rich {rich} is not"
                        ),
                    ));
                }
            }
            ToolRole::Scanner(scanner) => {
                within(name, "scanner.scan_s", scanner.scan_s, 1, MAX_TOOL_SECONDS)?;
                within(
                    name,
                    "scanner.sound_s",
                    scanner.sound_s,
                    1,
                    MAX_TOOL_SECONDS,
                )?;
                within(name, "scanner.mast_cm", scanner.mast_cm, 1, MAX_MAST_CM)?;
                within(name, "scanner.scan_cu", scanner.scan_cu, 1, MAX_TOOL_CU)?;
                within(name, "scanner.sound_cu", scanner.sound_cu, 1, MAX_TOOL_CU)?;
                let (quiet, sound, scan) = (
                    scanner.quiet_radius_cells,
                    scanner.sound_radius_cells,
                    scanner.scan_radius_cells,
                );
                if !(1 <= quiet && quiet <= sound && sound <= scan && scan <= MAX_RADIUS_CELLS) {
                    return Err(refuse(
                        name,
                        format!(
                            "scanner radii are 1 <= quiet <= sound <= scan <= {MAX_RADIUS_CELLS} \
                             cells per axis; quiet {quiet}, sound {sound}, scan {scan} is not"
                        ),
                    ));
                }
            }
            ToolRole::Battery(battery) => {
                within(
                    name,
                    "battery.capacity_cu",
                    battery.capacity_cu,
                    1,
                    MAX_TOOL_CU,
                )?;
                within(
                    name,
                    "battery.bay_recharge_cu_per_ic_hour",
                    battery.bay_recharge_cu_per_ic_hour,
                    1,
                    MAX_TOOL_CU,
                )?;
                within(
                    name,
                    "battery.drive_cu_per_nominal_cell",
                    battery.drive_cu_per_nominal_cell,
                    1,
                    MAX_TOOL_CU,
                )?;
                within(
                    name,
                    "battery.reserve_permille",
                    battery.reserve_permille,
                    1,
                    999,
                )?;
            }
        }
        Ok(())
    }

    /// The drill's figures, when this tool is a drill.
    #[must_use]
    pub fn drill(&self) -> Option<&DrillDef> {
        match &self.role {
            ToolRole::Drill(drill) => Some(drill),
            ToolRole::Scanner(_) | ToolRole::Battery(_) => None,
        }
    }

    /// The mast's figures, when this tool is a scanner.
    #[must_use]
    pub fn scanner(&self) -> Option<&ScannerDef> {
        match &self.role {
            ToolRole::Scanner(scanner) => Some(scanner),
            ToolRole::Drill(_) | ToolRole::Battery(_) => None,
        }
    }

    /// The battery's figures, when this tool is a battery.
    #[must_use]
    pub fn battery(&self) -> Option<&BatteryDef> {
        match &self.role {
            ToolRole::Battery(battery) => Some(battery),
            ToolRole::Drill(_) | ToolRole::Scanner(_) => None,
        }
    }
}

/// **One tool's canonical digest** under [`TOOLS_DIGEST_DOMAIN`]: the document
/// parsed and canonically encoded, so formatting is not content and a changed
/// figure moves the pin.
///
/// # Errors
///
/// [`DefinitionError`] for bytes that are not JSON or will not encode.
pub fn tool_digest(bytes: &str) -> Result<DefinitionDigest, DefinitionError> {
    let value: serde_json::Value =
        serde_json::from_str(bytes).map_err(|error| refuse("tool", error.to_string()))?;
    DefinitionDigest::for_canonical_value(TOOLS_DIGEST_DOMAIN, &value)
        .map_err(|error| refuse("tool", error.to_string()))
}

/// **The compiled-in tools** ([`super::content::TOOL_SOURCES`]), each loaded and
/// checked, by id; an id two files declare is refused naming both.
///
/// # Errors
///
/// As [`ToolDef::load`] — a build error, since `crates/tacsim/tests/content.rs`
/// pins every tool.
pub fn tools_table() -> Result<BTreeMap<CanonicalId, ToolDef>, DefinitionError> {
    let mut out: BTreeMap<CanonicalId, ToolDef> = BTreeMap::new();
    let mut files: BTreeMap<CanonicalId, &str> = BTreeMap::new();
    for (name, bytes) in super::content::TOOL_SOURCES {
        let tool = ToolDef::load(name, bytes)?;
        if let Some(first) = files.get(&tool.id) {
            return Err(refuse(
                name,
                format!("tool `{}` is also declared by {first}", tool.id),
            ));
        }
        files.insert(tool.id.clone(), name);
        out.insert(tool.id.clone(), tool);
    }
    Ok(out)
}
