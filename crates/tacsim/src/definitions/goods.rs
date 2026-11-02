//! **The trade run's goods table** (doc 18 cut IC2; doc 18 §4.3 "The goods
//! table" and "The validator"; D-18-13 as amended by R7, D-18-28 as replaced by
//! R7, D-18-39, D-18-40).
//!
//! Sixteen sale items in one tacsim-owned compiled-in document,
//! `crates/tacsim/content/market/goods.json`: eleven from doc 10 S4A's ten
//! families (doc 19 cut A.8 added `instrument_boards` and `carbon_feedstock`, the
//! moon ports' goods; v2; doc 20 cut SM.3a added the four run-of-mine ores,
//! `metal_ore`, `ice_ore`, `carbon_ore` and `strategic_ore`, under their own
//! stowage tag `stow.ore`, which a terrain vehicle's hopper and a Dray-P's hold
//! accept and a standard Dray's does not — doc 20 §7.3, Q-20-1; v3; doc 36 cut GC.7 added
//! the four munitions goods, `ammo_ap_30`, `ammo_ball_762`, `ammo_he_105` and `ammo_missile`,
//! each naming the ammunition family it loads; v4) and one
//! aggregate (`provisions`, class `ship_supply`, no family; Q-18-3). A top-level `measures` registry (`si.mass_kg` → kg, `si.volume_l` →
//! L) names doc 06 D3's measures, and every row carries its binding mass **and**
//! packed volume per unit, so a later measure is a data key on both sides and
//! the capacity function ([`crate::port::capacity`]) never collapses mass and
//! volume into one "fullness".
//!
//! # Why a typed document and not a pack (lead ruling R12)
//!
//! The table is the one source for two readings: the market's `CatalogEntry`
//! rows ([`GoodsTable::catalog_entries`], loaded by the port applier of cut IC8
//! through `mush_market::api::load_pack`) and the capacity check. Neither is
//! world state the content latch applies, so — the `resources.json` precedent
//! ([`super::resources`]) — it is a strict typed document, compiled in and
//! pinned by its canonical digest under [`GOODS_DIGEST_DOMAIN`]
//! (`crates/tacsim/tests/content.rs`). Physics never goes into `CommodityDef`
//! nor into the market catalog wire, which drops unknown fields.
//!
//! No row, note or derivation here names a mix or a best load (D-18-39): the
//! table states per-unit figures, and the optimum is the captain's skill.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, DefinitionDigest};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::logistics::{LogisticsDef, MASS_KG};
use super::resources::FAMILIES;
use super::{DefinitionError, MAX_LABEL_BYTES, check_label, check_note};

/// `crates/tacsim/content/market/goods.json`.
pub const GOODS_JSON: &str = include_str!("../../content/market/goods.json");

/// The market catalog pack name the derived rows carry (lead ruling R12).
pub const GOODS_PACK: &str = "tacsim.goods";

/// The domain the table's canonical digest is taken under; the encoding
/// version rides it, as a pack digest's does.
pub const GOODS_DIGEST_DOMAIN: &str = "tacsim.goods.v1";

/// The most measures the registry may name.
pub const MAX_MEASURES: usize = 8;

/// The most rows the table may carry.
pub const MAX_GOODS: usize = 32;

/// The longest ledger or quote unit word, bytes.
pub const MAX_UNIT_BYTES: usize = 16;

/// The largest per-unit figure of any measure: the largest pool
/// (`mush_inventory::MAX_POOL_QUANTITY`) times it still fits `u64` (doc 18
/// §4.3 "The validator": the largest stock × per-unit fits u64).
pub const MAX_PER_UNIT: u64 = u64::MAX / mush_inventory::MAX_POOL_QUANTITY.unsigned_abs();

/// The largest display reference, major units: `× 10^CREDIT_SCALE` still fits
/// `i64` with room to spare.
pub const MAX_REFERENCE_MAJOR: u64 = 1_000_000_000;

/// **The goods table** (doc 18 §4.3).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoodsTable {
    /// The document's version. A changed row is a new version.
    pub version: u32,
    /// The measure registry, `1..=8`, `si.mass_kg` always among them.
    pub measures: Vec<MeasureDef>,
    /// The rows, `1..=32`, in authored order (doc 18 §4.3's table order).
    pub goods: Vec<GoodDef>,
}

/// **One registered measure** (doc 06 D3's measure ids).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureDef {
    /// The measure id (`si.mass_kg`).
    pub id: CanonicalId,
    /// The unit a per-unit figure is in (`kg`).
    pub unit: SmolStr,
    /// The word player text uses for it (`mass`, `packed volume`; never
    /// "volume" alone, which is mush-market's maker budget).
    pub word: String,
}

/// **One good** (doc 18 §4.3's table, one row).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoodDef {
    /// The inventory commodity id (`metals`), which is also the market
    /// catalogue code.
    pub id: CanonicalId,
    /// The human-facing name (`metal concentrate`).
    pub label: String,
    /// The commodity family, one of doc 10 S4A's ten; `None` only for a
    /// non-trade class.
    pub family: Option<CanonicalId>,
    /// Trade good, or a ship's supply.
    pub class: GoodClass,
    /// The unit the inventory counts in (`t`, `crew_day`).
    pub ledger_unit: SmolStr,
    /// The unit a quote is per (`t`, `crew_day`).
    pub quote_unit: SmolStr,
    /// Measure id → the per-unit figure in that measure's unit.
    pub measures: BTreeMap<CanonicalId, u64>,
    /// The stowage tag a compartment must accept (`stow.bulk`).
    pub stowage: CanonicalId,
    /// What the passage burns it as, if anything.
    pub consumable: Option<Consumable>,
    /// **A display reference** copied from the origin port's posted ask at
    /// authoring time; never read by passage, fill, clamp, share or settlement
    /// arithmetic; the port listing (IC3/IC8, D-18-49) is the price. It becomes
    /// the derived `CatalogEntry.baseline`, which mush-market itself never
    /// charges.
    pub reference_major: u64,
    /// **What a munitions good loads** (doc 36 cut GC.7, Q-36-36): present exactly when the
    /// family is `munitions`. Skipped when absent, so every other row encodes as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub munitions: Option<MunitionsDef>,
    /// Why these numbers; never a mix or a best load (D-18-39).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// The most rounds one unit of a munitions good may hold.
pub const MAX_ROUNDS_PER_UNIT: u32 = 10_000;

/// **The ammunition a munitions good is** (doc 36 cut GC.7, Q-36-36). The link runs from the
/// good to the ammunition entry, so no ammunition pack (nor its digest) changes: a rearm
/// draws `ceil(rounds / rounds_per_unit)` units of the good for a magazine of `ammunition`.
/// Beam charge (`tacsim.ammunition.laser_charge`) is no good: it is a cell the workshop's
/// power recharges at the tariff's per-round labour, since heat, not the magazine, is the
/// laser's limit (16b cut X.1).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MunitionsDef {
    /// The ammunition entry (`tacsim.ammunition.he_105`).
    pub ammunition: CanonicalId,
    /// Rounds in one unit of the good, `1..=`[`MAX_ROUNDS_PER_UNIT`].
    pub rounds_per_unit: u32,
}

/// The munitions family's id (doc 10 S4A).
pub const MUNITIONS_FAMILY: &str = "munitions";

/// A good's class.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoodClass {
    /// A trade good, of one of the ten families.
    Trade,
    /// A ship's supply (provisions).
    ShipSupply,
}

impl GoodClass {
    /// The class word, as the table writes it and the catalog tags carry it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            GoodClass::Trade => "trade",
            GoodClass::ShipSupply => "ship_supply",
        }
    }
}

/// What a passage burns a good as (doc 18 §4.2 "Propellant and provisions as
/// consumables", D-18-11).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Consumable {
    /// Reaction mass: the departure burn.
    Propellant,
    /// Crew-days: occupants × each leg's whole IC days.
    Provisions,
}

impl GoodDef {
    /// The per-unit figure in `measure`, when the row declares it.
    #[must_use]
    pub fn per_unit(&self, measure: &CanonicalId) -> Option<u64> {
        self.measures.get(measure).copied()
    }

    /// **Kilograms per unit** — the burn and payload accessor (doc 18 §4.3:
    /// `mass_kg_per_unit` is derived, never authored twice).
    #[must_use]
    pub fn mass_kg_per_unit(&self) -> Option<u64> {
        self.measures
            .iter()
            .find(|(measure, _)| measure.as_str() == MASS_KG)
            .map(|(_, kg)| *kg)
    }
}

/// A row-level refusal: the document and the good, then what is wrong.
fn row_error(name: &str, id: &CanonicalId, detail: impl Into<String>) -> DefinitionError {
    DefinitionError::value(&format!("{name}: {id}"), detail)
}

impl GoodsTable {
    /// **Decode a table and re-run every bound** (doc 18 §4.3 "The validator").
    ///
    /// # Errors
    ///
    /// [`DefinitionError`] naming the document (and the good, for a row) and
    /// what is wrong.
    pub fn load(name: &str, bytes: &str) -> Result<GoodsTable, DefinitionError> {
        let table: GoodsTable = serde_json::from_str(bytes)
            .map_err(|error| DefinitionError::value(name, error.to_string()))?;
        if table.version == 0 {
            return Err(DefinitionError::value(name, "version 0 is reserved"));
        }
        if table.measures.is_empty() || table.measures.len() > MAX_MEASURES {
            return Err(DefinitionError::value(
                name,
                format!(
                    "the goods table registers 1 to {MAX_MEASURES} measures; {} is not",
                    table.measures.len()
                ),
            ));
        }
        let mut registered: BTreeSet<&CanonicalId> = BTreeSet::new();
        for measure in &table.measures {
            if !registered.insert(&measure.id) {
                return Err(DefinitionError::value(
                    name,
                    format!("measure `{}` is registered twice", measure.id),
                ));
            }
            for (field, text) in [("unit", measure.unit.as_str()), ("word", &measure.word)] {
                if text.trim().is_empty() || text.len() > MAX_LABEL_BYTES {
                    return Err(DefinitionError::value(
                        name,
                        format!(
                            "measure `{}`'s {field} is 1 to {MAX_LABEL_BYTES} bytes; {:?} is not",
                            measure.id, text
                        ),
                    ));
                }
            }
        }
        if !registered.iter().any(|id| id.as_str() == MASS_KG) {
            return Err(DefinitionError::value(
                name,
                format!("the goods table registers no {MASS_KG}; every good weighs something"),
            ));
        }
        if table.goods.is_empty() || table.goods.len() > MAX_GOODS {
            return Err(DefinitionError::value(
                name,
                format!(
                    "the goods table carries 1 to {MAX_GOODS} goods; {} is not",
                    table.goods.len()
                ),
            ));
        }
        let mut seen: BTreeSet<&CanonicalId> = BTreeSet::new();
        for good in &table.goods {
            let id = &good.id;
            if !seen.insert(id) {
                return Err(DefinitionError::value(
                    name,
                    format!("good `{id}` is declared twice"),
                ));
            }
            let entry = format!("{name}: {id}");
            check_label(&entry, &good.label)?;
            check_note(&entry, &good.note)?;
            match (&good.family, good.class) {
                (None, GoodClass::Trade) => {
                    return Err(row_error(name, id, "a trade good names its family"));
                }
                (Some(family), _) if !FAMILIES.contains(&family.as_str()) => {
                    return Err(row_error(
                        name,
                        id,
                        format!("`{family}` is not one of doc 10 S4A's ten families"),
                    ));
                }
                _ => {}
            }
            for (field, unit) in [
                ("ledger_unit", &good.ledger_unit),
                ("quote_unit", &good.quote_unit),
            ] {
                if unit.trim().is_empty() || unit.len() > MAX_UNIT_BYTES {
                    return Err(row_error(
                        name,
                        id,
                        format!("{field} is 1 to {MAX_UNIT_BYTES} bytes; {unit:?} is not"),
                    ));
                }
            }
            for (measure, per_unit) in &good.measures {
                if !registered.contains(measure) {
                    return Err(row_error(
                        name,
                        id,
                        format!("measure `{measure}` is not registered"),
                    ));
                }
                if *per_unit == 0 || *per_unit > MAX_PER_UNIT {
                    return Err(row_error(
                        name,
                        id,
                        format!("`{measure}` per unit is 1 to {MAX_PER_UNIT}; {per_unit} is not"),
                    ));
                }
            }
            if good.mass_kg_per_unit().is_none() {
                return Err(row_error(
                    name,
                    id,
                    format!("declares no {MASS_KG}; every good weighs something"),
                ));
            }
            let is_munitions = good
                .family
                .as_ref()
                .is_some_and(|family| family.as_str() == MUNITIONS_FAMILY);
            match &good.munitions {
                None if is_munitions => {
                    return Err(row_error(
                        name,
                        id,
                        "a munitions good names the ammunition it loads",
                    ));
                }
                Some(_) if !is_munitions => {
                    return Err(row_error(
                        name,
                        id,
                        "only a munitions good names an ammunition",
                    ));
                }
                Some(munitions)
                    if munitions.rounds_per_unit == 0
                        || munitions.rounds_per_unit > MAX_ROUNDS_PER_UNIT =>
                {
                    return Err(row_error(
                        name,
                        id,
                        format!(
                            "rounds_per_unit is 1 to {MAX_ROUNDS_PER_UNIT}; {} is not",
                            munitions.rounds_per_unit
                        ),
                    ));
                }
                _ => {}
            }
            if good.reference_major > MAX_REFERENCE_MAJOR {
                return Err(row_error(
                    name,
                    id,
                    format!(
                        "reference_major is at most {MAX_REFERENCE_MAJOR}; {} is not",
                        good.reference_major
                    ),
                ));
            }
        }
        Ok(table)
    }

    /// One good.
    #[must_use]
    pub fn good(&self, id: &CanonicalId) -> Option<&GoodDef> {
        self.goods.iter().find(|good| &good.id == id)
    }

    /// **The munitions good that loads `ammunition`** (doc 36 cut GC.7), if one does.
    #[must_use]
    pub fn munitions_for(&self, ammunition: &str) -> Option<&GoodDef> {
        self.goods.iter().find(|good| {
            good.munitions
                .as_ref()
                .is_some_and(|m| m.ammunition.as_str() == ammunition)
        })
    }

    /// Whether `id` is a registered measure.
    #[must_use]
    pub fn is_measure(&self, id: &CanonicalId) -> bool {
        self.measures.iter().any(|measure| &measure.id == id)
    }

    /// **The cross-check against one hull** (doc 18 §4.3 "The validator"):
    /// every compartment limit is a registered measure; every accepted tag is
    /// carried by some good; every good a compartment accepts carries every
    /// measure that compartment limits; and where a compartment accepts exactly
    /// one tag carried by exactly one good and limits two or more measures, the
    /// limits agree on the unit count (the locker: 480 / 4 = 1,200 / 10 = 120
    /// crew-days).
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] under `entry` (the hull), naming the
    /// compartment, the good and the measure.
    pub fn check_stowage(
        &self,
        entry: &str,
        logistics: &LogisticsDef,
    ) -> Result<(), DefinitionError> {
        for (compartment, def) in &logistics.stowage {
            if let Some(measure) = def.limits.keys().find(|measure| !self.is_measure(measure)) {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "compartment `{compartment}` limits `{measure}`, which the goods table \
                         does not register"
                    ),
                ));
            }
            for tag in &def.accepts {
                if !self.goods.iter().any(|good| &good.stowage == tag) {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "compartment `{compartment}` accepts `{tag}`, which no good carries"
                        ),
                    ));
                }
            }
            let stowed: Vec<&GoodDef> = self
                .goods
                .iter()
                .filter(|good| def.accepts.contains(&good.stowage))
                .collect();
            for good in &stowed {
                if let Some(measure) = def
                    .limits
                    .keys()
                    .find(|measure| good.per_unit(measure).is_none())
                {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "{} is stowed in {compartment}, which limits {measure}, and declares \
                             no {measure}",
                            good.id
                        ),
                    ));
                }
            }
            // The locker rule: one tag, one good, two or more limits — the
            // limits must say the same unit count.
            if def.accepts.len() == 1
                && def.limits.len() >= 2
                && let [good] = stowed.as_slice()
            {
                let mut floors: Vec<(&CanonicalId, u64)> = Vec::new();
                for (measure, limit) in &def.limits {
                    let per_unit = good.per_unit(measure).unwrap_or(0);
                    if per_unit == 0 {
                        continue;
                    }
                    floors.push((measure, limit / per_unit));
                }
                if let Some(((first, units), rest)) = floors.split_first()
                    && let Some((other, other_units)) =
                        rest.iter().find(|(_, count)| count != units)
                {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "compartment `{compartment}` holds {units} units of {} by {first} \
                             and {other_units} by {other}; the limits agree on the unit count",
                            good.id
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// **The market catalog rows** (doc 18 §4.3; lead ruling R12): one
    /// lot-represented, stackable, legal `CatalogEntry` per row, under ruleset
    /// `tacsim`, pack [`GOODS_PACK`], the table's version, the row's label, the
    /// reference in minor units at [`crate::CREDIT_SCALE`] as the (display-only)
    /// baseline, and the class word plus the family as tags. Built through
    /// `CatalogEntry`'s own wire, so its invariants run. Loading them is the
    /// port applier's (cut IC8); nothing here calls the market.
    ///
    /// # Errors
    ///
    /// [`DefinitionError`] when a reference overflows minor units or the
    /// market's wire refuses a row.
    pub fn catalog_entries(&self) -> Result<Vec<mush_market::CatalogEntry>, DefinitionError> {
        let scale = 10_u64
            .checked_pow(u32::from(crate::CREDIT_SCALE))
            .ok_or_else(|| DefinitionError::value(GOODS_PACK, "the credit scale overflows"))?;
        let mut out = Vec::with_capacity(self.goods.len());
        for good in &self.goods {
            let baseline = good
                .reference_major
                .checked_mul(scale)
                .and_then(|minor| i64::try_from(minor).ok())
                .ok_or_else(|| {
                    row_error(
                        GOODS_PACK,
                        &good.id,
                        "reference_major overflows minor units",
                    )
                })?;
            let mut tags = vec![good.class.as_str().to_string()];
            if let Some(family) = &good.family {
                tags.push(family.to_string());
            }
            let value = serde_json::json!({
                "catalog": { "ruleset": "tacsim", "code": good.id.as_str() },
                "pack": GOODS_PACK,
                "definition_version": self.version,
                "name": good.label,
                "baseline": baseline,
                "legality": "legal",
                "representation": "lot",
                "stackable": true,
                "tags": tags,
            });
            let entry: mush_market::CatalogEntry = serde_json::from_value(value)
                .map_err(|error| row_error(GOODS_PACK, &good.id, error.to_string()))?;
            out.push(entry);
        }
        Ok(out)
    }
}

/// **The table's canonical digest** under [`GOODS_DIGEST_DOMAIN`]: the
/// document parsed and canonically encoded, so formatting is not content and a
/// changed figure moves the pin.
///
/// # Errors
///
/// [`DefinitionError`] for bytes that are not JSON or will not encode.
pub fn goods_digest(bytes: &str) -> Result<DefinitionDigest, DefinitionError> {
    let value: serde_json::Value = serde_json::from_str(bytes)
        .map_err(|error| DefinitionError::value("goods.json", error.to_string()))?;
    DefinitionDigest::for_canonical_value(GOODS_DIGEST_DOMAIN, &value)
        .map_err(|error| DefinitionError::value("goods.json", error.to_string()))
}

/// **The compiled-in goods table**, loaded and checked.
///
/// # Errors
///
/// As [`GoodsTable::load`] — a build error, since
/// `crates/tacsim/tests/content.rs` pins the table.
pub fn goods_table() -> Result<GoodsTable, DefinitionError> {
    GoodsTable::load("goods.json", GOODS_JSON)
}
