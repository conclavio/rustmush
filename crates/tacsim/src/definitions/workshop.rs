//! **The workshop tariff** (doc 36 cut GC.7, Q-36-31): the one schedule every TacSim workshop
//! and refit bay quotes from, as content.
//!
//! `crates/tacsim/content/market/workshop_tariff.json` is a strict typed document compiled in,
//! as the goods table is ([`super::goods`]): it is read by the quote and never applied as
//! world state. Its `tariff` is `mush_tactics`' [`WorkshopFacilityTariff`] field for field, so
//! the engine owns the arithmetic and this file only the numbers. An operator never names a
//! fee: a port's authored workshop (`crate::port::def::WorkshopDef`) and a bay offer each carry
//! a multiplier within the schedule's bounds, and the quote is the schedule at that multiplier.
//!
//! The numbers are provisional (the owner's economy rule of 2026-10-24) and their basis is the
//! document's `note`: one IC hour of bay labour is 100 cr, the core-fitting fee.

use mush_core::DefinitionDigest;
use mush_tactics::field_ops::WorkshopFacilityTariff;
use serde::{Deserialize, Serialize};

use super::{DefinitionError, check_note};

/// `crates/tacsim/content/market/workshop_tariff.json`.
pub const WORKSHOP_TARIFF_JSON: &str = include_str!("../../content/market/workshop_tariff.json");

/// The domain the document's canonical digest is taken under.
pub const WORKSHOP_TARIFF_DIGEST_DOMAIN: &str = "tacsim.workshop_tariff.v1";

/// **The tariff document**.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkshopTariffDoc {
    /// The document's version. A changed figure is a new version.
    pub version: u32,
    /// The schedule.
    pub tariff: WorkshopFacilityTariff,
    /// Why these numbers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl WorkshopTariffDoc {
    /// Decode a document and run the schedule's own bounds.
    ///
    /// # Errors
    ///
    /// [`DefinitionError`] naming the document and what is wrong.
    pub fn load(name: &str, bytes: &str) -> Result<WorkshopTariffDoc, DefinitionError> {
        let doc: WorkshopTariffDoc = serde_json::from_str(bytes)
            .map_err(|error| DefinitionError::value(name, error.to_string()))?;
        if doc.version == 0 {
            return Err(DefinitionError::value(name, "version 0 is reserved"));
        }
        check_note(name, &doc.note)?;
        doc.tariff
            .validate()
            .map_err(|error| DefinitionError::value(name, error.to_string()))?;
        Ok(doc)
    }
}

/// **The compiled-in tariff**, loaded and checked.
///
/// # Errors
///
/// As [`WorkshopTariffDoc::load`] — a build error, since the content tests pin it.
pub fn workshop_tariff() -> Result<WorkshopFacilityTariff, DefinitionError> {
    WorkshopTariffDoc::load("workshop_tariff.json", WORKSHOP_TARIFF_JSON).map(|doc| doc.tariff)
}

/// **The document's canonical digest** under [`WORKSHOP_TARIFF_DIGEST_DOMAIN`].
///
/// # Errors
///
/// [`DefinitionError`] for bytes that are not JSON or will not encode.
pub fn workshop_tariff_digest(bytes: &str) -> Result<DefinitionDigest, DefinitionError> {
    let value: serde_json::Value = serde_json::from_str(bytes)
        .map_err(|error| DefinitionError::value("workshop_tariff.json", error.to_string()))?;
    DefinitionDigest::for_canonical_value(WORKSHOP_TARIFF_DIGEST_DOMAIN, &value)
        .map_err(|error| DefinitionError::value("workshop_tariff.json", error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped schedule loads, and a refit at the operators' ceiling stays under the bay's
    /// fee ceiling (`crate::hardware::BAY_MAX_FEE_MINOR`).
    #[test]
    fn gc7_the_shipped_tariff_loads_and_a_refit_at_twice_fits_the_bay_ceiling() {
        let tariff = workshop_tariff().expect("the shipped tariff loads");
        let dear = tariff
            .with_multiplier_permille(tariff.max_multiplier_permille)
            .expect("the ceiling is a legal multiplier");
        assert!(
            i64::try_from(dear.labor_cost_replace_component).expect("fits")
                <= crate::hardware::BAY_MAX_FEE_MINOR
        );
        let mut broken: serde_json::Value =
            serde_json::from_str(WORKSHOP_TARIFF_JSON).expect("JSON");
        broken["tariff"]["max_multiplier_permille"] = 999.into();
        assert!(WorkshopTariffDoc::load("broken", &broken.to_string()).is_err());
    }
}
