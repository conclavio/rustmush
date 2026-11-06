//! **The class templates** (doc 39 cut IN.9b, §4.13; Q-39-14, Q-39-20, Q-39-35): `armour_cab` and
//! `walker_cab`, the `logistics` and `interior` blocks a simulator design takes at the catalogue
//! build so the yard can lay it up and a carrier can carry it.
//!
//! Each template names the designs it applies to (`content/industry/cab_templates.json`): the
//! line-tank family, the heavy and the scout take `armour_cab`; the two bipeds take `walker_cab`.
//! A template is a cab room with the console and a kit locker as the stowage, one berth and no
//! drive. It is applied to the catalogue's stored spec only, after the unit's binding and its mass
//! budget are made, so neither moves: a templated design's `dry_mass_t` is its field-ops mass
//! rounded up to whole tonnes for the readouts that print tonnes, and every mass the engine
//! reads (`crate::field_ops::dry_mass_kg_for_spec`: a tow, a bay's load, the partition) stays the
//! structural-point measure ([`LogisticsDef::derived_mass`]).
//!
//! The drone, the carrier, the target, the relay station and the capital hulls take none
//! (Q-39-20, Q-39-35). A design that authors either block itself and is named by a template is
//! refused, as is a template whose class does not match its design's.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::CanonicalId;
use serde::Deserialize;

use crate::definitions::unit::{UnitClass, UnitSpec};
use crate::definitions::{DefinitionError, InteriorDef, LogisticsDef, StowageDef};

/// The templates' table: `content/industry/cab_templates.json`.
pub const CAB_TEMPLATES_JSON: &str = include_str!("../../content/industry/cab_templates.json");

/// The table's version.
pub const CAB_TEMPLATES_VERSION: u32 = 1;

/// The armour template's id.
pub const ARMOUR_CAB: &str = "armour_cab";
/// The walker template's id.
pub const WALKER_CAB: &str = "walker_cab";

/// The table as authored.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    version: u32,
    #[serde(default)]
    #[allow(dead_code)] // the table's own prose
    note: String,
    templates: BTreeMap<String, CabTemplate>,
}

/// **One class template** as authored: the designs it applies to and its two blocks.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CabTemplate {
    /// The unit entries it applies to.
    pub applies: BTreeSet<CanonicalId>,
    /// The logistics block, its mass from the design.
    pub logistics: TemplateLogistics,
    /// The interior block.
    pub interior: InteriorDef,
}

/// **A template's logistics** — [`LogisticsDef`] without the dry mass, which is the design's.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateLogistics {
    /// The stowage compartments.
    pub stowage: BTreeMap<CanonicalId, StowageDef>,
    /// The berths.
    pub berths: u32,
    /// What one occupant weighs against the payload, kg.
    pub occupant_allowance_kg: u32,
    /// The payload rail, kg.
    pub payload_ceiling_kg: u64,
}

/// The class a template's designs must be.
fn class_of_template(id: &str) -> Option<UnitClass> {
    match id {
        ARMOUR_CAB => Some(UnitClass::Armour),
        WALKER_CAB => Some(UnitClass::Walker),
        _ => None,
    }
}

/// **The shipped templates**, checked: the version, the two known ids, no design named twice.
///
/// # Errors
///
/// A [`DefinitionError`] naming what is wrong.
pub fn templates() -> Result<BTreeMap<String, CabTemplate>, DefinitionError> {
    let table: Table = serde_json::from_str(CAB_TEMPLATES_JSON)
        .map_err(|error| DefinitionError::value("cab_templates.json", error.to_string()))?;
    if table.version != CAB_TEMPLATES_VERSION {
        return Err(DefinitionError::value(
            "cab_templates.json",
            format!("version {} is not {CAB_TEMPLATES_VERSION}", table.version),
        ));
    }
    let mut seen = BTreeSet::new();
    for (id, template) in &table.templates {
        if class_of_template(id).is_none() {
            return Err(DefinitionError::value(
                "cab_templates.json",
                format!("`{id}` is not a class template this build knows"),
            ));
        }
        for design in &template.applies {
            if !seen.insert(design.clone()) {
                return Err(DefinitionError::value(
                    "cab_templates.json",
                    format!("{design} takes two templates"),
                ));
            }
        }
    }
    Ok(table.templates)
}

/// **The template `design` takes**, with its id, if any.
#[must_use]
pub fn template_for<'t>(
    templates: &'t BTreeMap<String, CabTemplate>,
    design: &CanonicalId,
) -> Option<(&'t str, &'t CabTemplate)> {
    templates
        .iter()
        .find(|(_, template)| template.applies.contains(design))
        .map(|(id, template)| (id.as_str(), template))
}

/// **Apply `design`'s template to its stored spec**, when one names it: the logistics block at
/// the design's own mass ([`LogisticsDef::derived_mass`]) and the interior, each checked.
///
/// # Errors
///
/// A design that authors either block itself, one whose class is not the template's, or a block
/// that fails its own checks.
pub fn apply(
    templates: &BTreeMap<String, CabTemplate>,
    design: &CanonicalId,
    spec: &mut UnitSpec,
) -> Result<(), DefinitionError> {
    let Some((id, template)) = template_for(templates, design) else {
        return Ok(());
    };
    let entry = design.as_str();
    if spec.logistics.is_some() || spec.interior.is_some() {
        return Err(DefinitionError::value(
            entry,
            format!("authors its own logistics or interior and is named by the {id} template"),
        ));
    }
    if class_of_template(id) != Some(spec.class_of()) {
        return Err(DefinitionError::value(
            entry,
            format!("is not of the {id} template's class"),
        ));
    }
    let kg = crate::field_ops::dry_mass_kg_for_spec(spec);
    let dry_mass_t = u32::try_from(kg.div_ceil(1_000))
        .map_err(|_| DefinitionError::value(entry, "its dry mass is out of range"))?;
    let logistics = LogisticsDef {
        dry_mass_t,
        stowage: template.logistics.stowage.clone(),
        berths: template.logistics.berths,
        occupant_allowance_kg: template.logistics.occupant_allowance_kg,
        payload_ceiling_kg: template.logistics.payload_ceiling_kg,
        drive: None,
        tender: None,
        recovery_profile: crate::definitions::logistics::RecoveryProfile::default(),
        tow_cap_t: None,
        derived_mass: true,
    };
    logistics.check(entry)?;
    template.interior.check(entry, &logistics)?;
    spec.logistics = Some(logistics);
    spec.interior = Some(template.interior.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in9b_the_two_templates_load_and_name_nine_designs() {
        let templates = templates().expect("the shipped templates");
        assert_eq!(templates.len(), 2);
        let named: usize = templates.values().map(|t| t.applies.len()).sum();
        assert_eq!(
            named, 9,
            "the line-tank family, the heavy, the scout and the two bipeds"
        );
    }
}
