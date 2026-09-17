//! **The ten commodity families** (16b cut C.2, C4; doc 10 §12 S4A, doc 11 §2,
//! doc 16 §4 "Content packs").
//!
//! Doc 10 S4A fixes the list exactly: *raw: metals, volatiles, carbon feedstock,
//! strategic elements; refined: structural alloys, composites, electronics,
//! propellant; operational: industrial parts, munitions.* They arrive here as
//! `mush_economy::InstrumentDef` rows of kind `Commodity`, registered
//! create-only at boot beside the settlement credit, and are **shipped
//! unconsumed by the story**: no arena reads one, and nothing in cut C spends,
//! mints or holds a unit of anything.
//!
//! # Why this is a typed document and not a pack entry
//!
//! An `InstrumentDef` is `mush-economy`'s durable row with its own `revision`,
//! and registering one is [`crate::economy::ensure_instruments`]' create-only
//! job at boot. Wrapping it in a pack entry would give one row two identities —
//! the pack digest and the instrument revision — and put a create-only economy
//! write behind the content latch, which is neither where it lives nor where its
//! repair path (`@instrument/register`) is. So the file is a strict typed
//! document, compiled in and digest-pinned exactly as the packs beside it are,
//! and the ids are the same ten `CanonicalId`s the corridor fixture's
//! `commodity` entries already declare.

use mush_economy::{
    InstrumentDef, InstrumentDisplay, InstrumentId, InstrumentKind, InstrumentStatus,
    NegativePolicy, SymbolPosition,
};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::definitions::DefinitionError;

/// Doc 10 S4A's ten, in its own order: raw, refined, operational.
pub const FAMILIES: [&str; 10] = [
    "metals",
    "volatiles",
    "carbon_feedstock",
    "strategic_elements",
    "structural_alloys",
    "composites",
    "electronics",
    "propellant",
    "industrial_parts",
    "munitions",
];

/// One commodity family as an economy row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceRow {
    /// The instrument id, `tacsim.<family>`.
    pub id: SmolStr,
    /// The family id, one of [`FAMILIES`] — what a recipe, a deposit and a pool
    /// already name.
    pub family: SmolStr,
    /// The short code the economy renders.
    pub code: SmolStr,
    /// The human-facing name.
    pub label: String,
    /// The minor-unit scale. Zero: a commodity is counted in whole units.
    pub scale: u8,
    /// Why this row.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// The shipped resource table.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceTable {
    /// The document's version. A changed row is a new version.
    pub version: u32,
    /// The rows, one per family.
    pub resources: Vec<ResourceRow>,
}

impl ResourceTable {
    /// Decode a table and re-run every bound.
    ///
    /// # Errors
    ///
    /// [`DefinitionError`] naming the document and what is wrong: bytes that
    /// are not this shape, a family outside [`FAMILIES`], a repeat, or a list
    /// that is not the whole ten.
    pub fn load(name: &str, bytes: &str) -> Result<ResourceTable, DefinitionError> {
        let table: ResourceTable = serde_json::from_str(bytes)
            .map_err(|error| DefinitionError::value(name, error.to_string()))?;
        if table.version == 0 {
            return Err(DefinitionError::value(
                name,
                "version 0 is reserved".to_string(),
            ));
        }
        if table.resources.len() != FAMILIES.len() {
            return Err(DefinitionError::value(
                name,
                format!(
                    "doc 10 S4A fixes ten commodity families; {} is not ten",
                    table.resources.len()
                ),
            ));
        }
        let mut seen: Vec<&str> = Vec::new();
        for row in &table.resources {
            crate::definitions::check_label(name, &row.label)?;
            crate::definitions::check_note(name, &row.note)?;
            if !FAMILIES.contains(&row.family.as_str()) {
                return Err(DefinitionError::value(
                    name,
                    format!(
                        "`{}` is not one of doc 10 S4A's ten families {FAMILIES:?}",
                        row.family
                    ),
                ));
            }
            if seen.contains(&row.family.as_str()) {
                return Err(DefinitionError::value(
                    name,
                    format!("family `{}` is declared twice", row.family),
                ));
            }
            seen.push(row.family.as_str());
            if row.code.is_empty() || row.code.len() > 8 {
                return Err(DefinitionError::value(
                    name,
                    format!(
                        "`{}`'s code is 1 to 8 bytes; `{}` is not",
                        row.family, row.code
                    ),
                ));
            }
            if row.scale > 4 {
                return Err(DefinitionError::value(
                    name,
                    format!("`{}`'s scale is 0 to 4; {} is not", row.family, row.scale),
                ));
            }
            if InstrumentId::new(row.id.as_str()).is_none() {
                return Err(DefinitionError::value(
                    name,
                    format!("`{}` is not an instrument id", row.id),
                ));
            }
        }
        Ok(table)
    }

    /// The rows as economy instruments, in authored order.
    ///
    /// A commodity is `transferable` and `NegativePolicy::Forbid`: nobody owes a
    /// tonne of metals into existence, and unlike the settlement credit there is
    /// no faucet to except (the credit is `SystemOnly` for exactly that reason),
    /// so the strictest policy is the true one. It renders with its code after
    /// the number, as the credit does.
    #[must_use]
    pub fn instruments(&self) -> Vec<InstrumentDef> {
        self.resources
            .iter()
            .filter_map(|row| {
                Some(InstrumentDef {
                    id: InstrumentId::new(row.id.as_str())?,
                    ruleset: SmolStr::new_static("tacsim"),
                    code: row.code.clone(),
                    kind: InstrumentKind::Commodity,
                    scale: row.scale,
                    transferable: true,
                    negative_policy: NegativePolicy::Forbid,
                    status: InstrumentStatus::Active,
                    display: Some(InstrumentDisplay {
                        symbol: row.code.clone(),
                        position: SymbolPosition::Suffix,
                    }),
                    revision: 1,
                })
            })
            .collect()
    }
}
