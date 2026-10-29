//! Strict authored quantities and schedules, independent of economic ownership.
use crate::definitions::DefinitionError;
use mush_economy::core_integration::CoreClass;
use serde::{Deserialize, Serialize};

pub const PACK: &str = "tacsim.pack.governed_hardware";
pub const VERSION: u32 = 1;
pub const CATALOGUE: &str = "tacsim.hardware.catalogue";
pub const INTEGRATE: &str = "core_integrate";
pub const REFIT: &str = "core_refit";
/// The ungoverning refit's recipe (doc 17h cut M8b C2).
pub const UNGOVERN: &str = "core_ungovern";
pub const CLASSES: [CoreClass; 3] = [CoreClass::I, CoreClass::II, CoreClass::III];

fn bad(detail: &str) -> DefinitionError {
    DefinitionError::value("governed hardware content", detail)
}

/// Three distinct workflows. None is a pooled commodity recipe.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareProcess {
    /// One certified quantity-one die produces one certified titled core.
    Integrate,
    /// One delivered certified core replaces the designated governed component.
    Refit,
    /// **The ungoverning refit** (doc 17h cut M8b C2; doc 17 §10, D-UM-42): one
    /// delivered core replaces the component as its ungoverned twin, at a
    /// facility holding `core_ungovern` and not `core_refit`.
    Ungovern,
}
impl HardwareProcess {
    pub const fn recipe_id(self) -> &'static str {
        match self {
            Self::Integrate => INTEGRATE,
            Self::Refit => REFIT,
            Self::Ungovern => UNGOVERN,
        }
    }
    pub const fn facility_capability(self) -> &'static str {
        match self {
            Self::Integrate => "core_integration",
            Self::Refit => "core_refit",
            Self::Ungovern => "core_ungovern",
        }
    }
}

/// The real recipe has certified input/output semantics; there are deliberately
/// no commodity, caller-selected grade or arbitrary output fields here.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HardwareRecipeDef {
    format: u16,
    label: String,
    process: HardwareProcess,
    /// Class I, II, III in order, at the actual Industry hourly cadence.
    work_units: [u64; 3],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeWire {
    format: u16,
    label: String,
    process: HardwareProcess,
    work_units: [u64; 3],
}
impl<'de> Deserialize<'de> for HardwareRecipeDef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = RecipeWire::deserialize(d)?;
        let value = Self {
            format: w.format,
            label: w.label,
            process: w.process,
            work_units: w.work_units,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}
impl HardwareRecipeDef {
    pub fn validate(&self) -> Result<(), DefinitionError> {
        if self.format != 1
            || self.label.trim().is_empty()
            || self.label.len() > crate::pack::MAX_LABEL_BYTES
            || self.work_units.iter().any(|n| !(1..=167).contains(n))
        {
            return Err(bad(
                "invalid recipe version, label or bounded hourly schedule",
            ));
        }
        Ok(())
    }
    pub fn label(&self) -> &str {
        &self.label
    }
    pub const fn process(&self) -> HardwareProcess {
        self.process
    }
    pub fn work_units(&self, class: CoreClass) -> u64 {
        self.work_units[class.catalog_index()]
    }
}

/// Versioned launch prices in centicredits, not rounded display strings.
/// The current native protocol fixes these same v1 prices. A pack cannot change
/// the displayed quote without also changing that financial protocol.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HardwareCatalogueDef {
    format: u16,
    instrument: String,
    scale: u8,
    tank: String,
    tank_minor: i64,
    core_minor: [i64; 3],
    integration_permille: u16,
    fitting_minor: i64,
    /// **The ungoverning refit's fee** (doc 17h cut M8b C7), centicredits.
    /// Provisional per the owner's economy rule: the fitting fee.
    ungovern_minor: i64,
    monthly_dies: [u16; 3],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogueWire {
    format: u16,
    instrument: String,
    scale: u8,
    tank: String,
    tank_minor: i64,
    core_minor: [i64; 3],
    integration_permille: u16,
    fitting_minor: i64,
    ungovern_minor: i64,
    monthly_dies: [u16; 3],
}
impl<'de> Deserialize<'de> for HardwareCatalogueDef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = CatalogueWire::deserialize(d)?;
        let value = Self {
            format: w.format,
            instrument: w.instrument,
            scale: w.scale,
            tank: w.tank,
            tank_minor: w.tank_minor,
            core_minor: w.core_minor,
            integration_permille: w.integration_permille,
            fitting_minor: w.fitting_minor,
            ungovern_minor: w.ungovern_minor,
            monthly_dies: w.monthly_dies,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}
impl HardwareCatalogueDef {
    pub fn validate(&self) -> Result<(), DefinitionError> {
        if self.format != 1
            || self.instrument != crate::economy::CREDIT
            || self.scale != crate::economy::CREDIT_SCALE
            || self.tank != crate::definitions::core_replacement::LINE_II
            || self.tank_minor != 10_000_000
            || self.integration_permille != 100
            || self.fitting_minor != mush_economy::core_refit::FITTING_MINOR
            || !(1..=self.tank_minor).contains(&self.ungovern_minor)
            || self.monthly_dies.iter().any(|n| !(1..=1024).contains(n))
        {
            return Err(bad(
                "catalogue differs from the v1 credit or fitting protocol",
            ));
        }
        for (index, (class, percent)) in CLASSES.into_iter().zip([3, 8, 25]).enumerate() {
            let reference = self.tank_minor.checked_mul(percent).map(|n| n / 100);
            if reference != Some(self.core_minor[index])
                || self.core_minor[index] != class.finished_minor()
                || self.integration_minor(class)? != class.integration_minor()
                || self.die_minor(class)? != class.die_minor()
            {
                return Err(bad(
                    "core prices differ from the tank percentages or native split",
                ));
            }
        }
        Ok(())
    }
    pub fn finished_minor(&self, class: CoreClass) -> i64 {
        self.core_minor[class.catalog_index()]
    }
    pub fn integration_minor(&self, class: CoreClass) -> Result<i64, DefinitionError> {
        self.finished_minor(class)
            .checked_mul(i64::from(self.integration_permille))
            .map(|n| n / 1000)
            .ok_or_else(|| bad("integration price overflow"))
    }
    pub fn die_minor(&self, class: CoreClass) -> Result<i64, DefinitionError> {
        self.finished_minor(class)
            .checked_sub(self.integration_minor(class)?)
            .ok_or_else(|| bad("die price overflow"))
    }
    pub const fn fitting_minor(&self) -> i64 {
        self.fitting_minor
    }
    /// The ungoverning refit's fee (doc 17h cut M8b C7).
    pub const fn ungovern_minor(&self) -> i64 {
        self.ungovern_minor
    }
    pub const fn monthly_dies(&self) -> [u16; 3] {
        self.monthly_dies
    }
}
