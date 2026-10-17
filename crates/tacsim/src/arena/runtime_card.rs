//! Immutable authored card/program definitions and bounded per-arena sharing.
//!
//! A snapshot stores each complete definition once. Unit references and active
//! cards carry its digest and profile index; immutable runtime data is hydrated
//! only after the catalog and every reference have been validated. A missing or
//! tampered definition is an error, never a preset/default reconstruction.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::DefinitionDigest;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::card::StockCard;
use super::core::CoreClass;
use super::schema::{
    AuthoredRule, CardField, CardSchema, CardSection, CardValue, CardValues, SchemaError, Widget,
    validate_envelope,
};

/// Maximum distinct definitions in an arena, including staff live replacements.
pub const MAX_CARD_DEFINITIONS: usize = 512;
/// Aggregate canonical definition bytes; whole-snapshot admission remains separate.
pub const MAX_CARD_CATALOG_BYTES: usize = 2 * 1024 * 1024;

/// A complete profile payload in a seal and shared catalog definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileDefinition {
    /// User-visible name, unique without regard to case in this definition.
    pub name: SmolStr,
    /// Every schema field, in its exact integer/closed-widget representation.
    pub values: CardValues,
    /// Ordered programs; think gates must stay attached to their native actions.
    pub rules: Vec<AuthoredRule>,
}

/// One immutable schema and complete profile set, shared by all matching hulls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardDefinition {
    /// The closed schema, stored once even when every profile uses it.
    pub schema: CardSchema,
    /// The class under whose declared profile/rule limits this was authored.
    pub declared_class: CoreClass,
    /// Explicit order; the first is the opening profile unless selection reorders it.
    pub profiles: Vec<ProfileDefinition>,
}

impl CardDefinition {
    /// Validate and obtain canonical identity; work occurs only at admission.
    ///
    /// # Errors
    /// Invalid complete cards, rules, names, capacities or encoded size.
    pub fn validate(&self) -> Result<DefinitionDigest, SchemaError> {
        let mut profiles = BTreeMap::new();
        let mut rules = BTreeMap::new();
        for profile in &self.profiles {
            if profiles
                .insert(profile.name.clone(), profile.values.clone())
                .is_some()
            {
                return Err(invalid("profiles", "duplicate profile name"));
            }
            rules.insert(profile.name.clone(), profile.rules.clone());
        }
        validate_envelope(
            &json(&self.schema)?,
            &serde_json::json!({}),
            &json(&profiles)?,
            &json(&rules)?,
            self.declared_class,
        )?;
        DefinitionDigest::for_canonical_value_with_limit(
            "tacsim.card-program.v1",
            &json(self)?,
            super::schema::MAX_SCHEMA_BYTES,
        )
        .map_err(|error| invalid("card definition", error.to_string()))
    }

    fn hydrate(&self, digest: DefinitionDigest) -> Result<Arc<HydratedDefinition>, SchemaError> {
        let schema = Arc::new(self.schema.clone());
        let mut profiles = Vec::with_capacity(self.profiles.len());
        for (index, profile) in self.profiles.iter().enumerate() {
            let card_bytes = serde_json::to_vec(&serde_json::json!({
                "schema":self.schema, "values":profile.values, "rules":profile.rules,
            }))
            .map_err(|_| invalid("card", "cannot encode canonical card"))?;
            // Reusable native/stock access is a derived adapter over complete
            // values. It is never an independent source of executable fields.
            let stock = stock_adapter(&profile.values);
            profiles.push(Arc::new(CardData {
                schema: schema.clone(),
                values: Arc::new(profile.values.clone()),
                rules: profile.rules.clone().into(),
                stock,
                card_bytes: card_bytes.into(),
                key: ProfileKey {
                    definition: digest,
                    index: u8::try_from(index)
                        .map_err(|_| invalid("profiles", "profile index overflow"))?,
                },
            }));
        }
        Ok(Arc::new(HydratedDefinition { profiles }))
    }
}

/// A stable reference to a complete profile, never a stock preset code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileKey {
    /// Canonical digest of the full schema/profile/program definition.
    pub definition: DefinitionDigest,
    /// Index in that immutable definition's ordered profile list.
    pub index: u8,
}

/// Hydrated immutable card data. Clones of runtime cards share this allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardData {
    schema: Arc<CardSchema>,
    values: Arc<CardValues>,
    rules: Arc<[AuthoredRule]>,
    stock: Option<StockCard>,
    card_bytes: Arc<[u8]>,
    key: ProfileKey,
}

/// A card's persisted key and skipped immutable cache, resolved before execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCard {
    key: ProfileKey,
    #[serde(skip)]
    data: Option<Arc<CardData>>,
}

impl PartialEq for RuntimeCard {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for RuntimeCard {}

impl RuntimeCard {
    /// Compact immutable identity, folded by the hot state hash.
    #[must_use]
    pub const fn key(&self) -> ProfileKey {
        self.key
    }

    /// Read an authored value without copying its schema or allocating a map.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&CardValue> {
        self.data.as_ref()?.values.get(key)
    }

    /// Every stored value for the shared own-card renderer.
    #[must_use]
    pub fn values(&self) -> Option<&CardValues> {
        Some(&self.data.as_ref()?.values)
    }

    /// Typed adapter for stock helpers; absent for a custom card's unrelated fields.
    #[must_use]
    pub fn stock(&self) -> Option<&StockCard> {
        self.data.as_ref()?.stock.as_ref()
    }

    /// Complete schema for every renderer; unavailable only before hydration.
    #[must_use]
    pub fn schema(&self) -> Option<&CardSchema> {
        Some(&self.data.as_ref()?.schema)
    }

    /// Ordered programs attached to this exact profile.
    #[must_use]
    pub fn rules(&self) -> Option<&[AuthoredRule]> {
        Some(&self.data.as_ref()?.rules)
    }

    /// Existing seal byte-array payload, precomputed once at admission.
    #[must_use]
    pub fn canonical_bytes(&self) -> Option<&[u8]> {
        Some(&self.data.as_ref()?.card_bytes)
    }

    /// Whether validated catalog data is installed for this persisted key.
    #[must_use]
    pub fn is_hydrated(&self) -> bool {
        self.data.is_some()
    }
}

#[derive(Clone, Debug)]
struct HydratedDefinition {
    profiles: Vec<Arc<CardData>>,
}

/// One serialized catalog and runtime cache per arena. Decode calls rebuild
/// before hydrating units; no getter auto-admits or reconstructs a definition.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardCatalog {
    definitions: BTreeMap<DefinitionDigest, CardDefinition>,
    #[serde(skip)]
    hydrated: BTreeMap<DefinitionDigest, Arc<HydratedDefinition>>,
    #[serde(skip)]
    encoded_bytes: usize,
}

impl CardCatalog {
    /// Number of distinct immutable definitions, regardless of hull count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    /// Whether any definition has been admitted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }

    /// Stable identities include unused definitions, which still consume capacity.
    pub fn digests(&self) -> impl Iterator<Item = DefinitionDigest> + '_ {
        self.definitions.keys().copied()
    }

    /// Aggregate canonical payload bytes (plus bounded digest-key overhead).
    #[must_use]
    pub const fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }

    /// Admit a complete definition atomically and deduplicate equal payloads.
    ///
    /// # Errors
    /// Semantic invalidity, a digest mismatch/collision or either catalog bound.
    pub fn admit(&mut self, definition: CardDefinition) -> Result<DefinitionDigest, SchemaError> {
        if self.definitions.len() != self.hydrated.len() {
            return Err(invalid(
                "catalog",
                "decoded catalog must be rebuilt before admission",
            ));
        }
        let digest = definition.validate()?;
        if let Some(existing) = self.definitions.get(&digest) {
            if existing != &definition {
                return Err(invalid("catalog", "definition digest collision"));
            }
            if !self.hydrated.contains_key(&digest) {
                return Err(invalid(
                    "catalog",
                    "decoded catalog must be rebuilt before admission",
                ));
            }
            return Ok(digest);
        }
        if self.definitions.len() >= MAX_CARD_DEFINITIONS {
            return Err(invalid("catalog", "maximum 512 distinct card definitions"));
        }
        let bytes = definition_size(&definition)?;
        let total = self
            .encoded_bytes
            .checked_add(bytes)
            .ok_or_else(|| invalid("catalog", "byte count overflow"))?;
        // The outer object plus an empty definitions map takes 18 bytes.
        if total.saturating_add(18) > MAX_CARD_CATALOG_BYTES {
            return Err(invalid(
                "catalog",
                "maximum 2097152 aggregate card definition bytes",
            ));
        }
        let hydrated = definition.hydrate(digest)?;
        self.definitions.insert(digest, definition);
        self.hydrated.insert(digest, hydrated);
        self.encoded_bytes = total;
        Ok(digest)
    }

    /// Validate every decoded key/payload and build immutable data before commit.
    ///
    /// # Errors
    /// Any corrupt definition or aggregate bound. The old catalog is untouched.
    pub fn rebuild(&mut self) -> Result<(), SchemaError> {
        let mut staged = Self::default();
        for (key, definition) in &self.definitions {
            let actual = staged.admit(definition.clone())?;
            if &actual != key {
                return Err(invalid(
                    "catalog",
                    "stored digest does not match canonical definition",
                ));
            }
        }
        *self = staged;
        Ok(())
    }

    /// Resolve one persisted key; absent keys/indices are explicit errors.
    ///
    /// # Errors
    /// Missing/corrupt catalog or profile index.
    pub fn resolve(&self, key: ProfileKey) -> Result<RuntimeCard, SchemaError> {
        let data = self
            .hydrated
            .get(&key.definition)
            .and_then(|definition| definition.profiles.get(usize::from(key.index)))
            .cloned()
            .ok_or_else(|| invalid("profile", "missing validated catalog definition or profile"))?;
        if data.key != key {
            return Err(invalid(
                "profile",
                "runtime catalog cache identity mismatch",
            ));
        }
        Ok(RuntimeCard {
            key,
            data: Some(data),
        })
    }

    /// Install data for a decoded active card, preserving its exact key.
    ///
    /// # Errors
    /// Same as resolve; no default is substituted.
    pub fn hydrate_card(&self, card: &mut RuntimeCard) -> Result<(), SchemaError> {
        *card = self.resolve(card.key)?;
        Ok(())
    }

    /// Full definition for custody/seal/export or read-only profile rendering.
    #[must_use]
    pub fn definition(&self, digest: DefinitionDigest) -> Option<&CardDefinition> {
        self.definitions.get(&digest)
    }
}

/// Small named binding carried by a hull's seal reference and the cockpit frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileBinding {
    /// Canonical user-facing name.
    pub name: SmolStr,
    /// Complete immutable payload in the shared catalog.
    pub key: ProfileKey,
}

/// A bounded, explicitly ordered set of immutable catalog profile bindings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuntimeProfiles(Vec<ProfileBinding>);

impl RuntimeProfiles {
    /// An empty marker for a hull or seal that has not been bound yet.
    /// This is not an admissible profile selection and fails validation.
    pub(crate) fn unbound() -> Self {
        Self(Vec::new())
    }

    /// Bind selected names once from a validated catalog; no preset lookup.
    /// An empty choice means the first profile alone, preserving opening defaults.
    ///
    /// # Errors
    /// Missing/duplicate names, unknown definitions or a set outside 1..4.
    pub fn bind(
        catalog: &CardCatalog,
        definition: DefinitionDigest,
        names: &[SmolStr],
    ) -> Result<Self, SchemaError> {
        let payload = catalog
            .definition(definition)
            .ok_or_else(|| invalid("profiles", "unknown definition"))?;
        let names: Vec<_> = if names.is_empty() {
            payload
                .profiles
                .first()
                .map(|p| vec![p.name.clone()])
                .unwrap_or_default()
        } else {
            names.to_vec()
        };
        if names.is_empty() || names.len() > 4 {
            return Err(invalid("profiles", "expected 1 through 4 profiles"));
        }
        let mut seen = BTreeSet::new();
        let mut entries = Vec::with_capacity(names.len());
        for name in names {
            if !seen.insert(name.to_ascii_lowercase()) {
                return Err(invalid("profiles", "duplicate profile"));
            }
            let (index, profile) = payload
                .profiles
                .iter()
                .enumerate()
                .find(|(_, profile)| profile.name.eq_ignore_ascii_case(&name))
                .ok_or_else(|| invalid("profiles", "profile not in saved revision"))?;
            let key = ProfileKey {
                definition,
                index: u8::try_from(index)
                    .map_err(|_| invalid("profiles", "profile index overflow"))?,
            };
            catalog.resolve(key)?;
            entries.push(ProfileBinding {
                name: profile.name.clone(),
                key,
            });
        }
        Ok(Self(entries))
    }

    /// Verify a decoded binding set against its catalog and effective class.
    ///
    /// # Errors
    /// Capacity, mixed definitions, unknown/mismatched names or duplicate keys.
    pub fn validate(&self, catalog: &CardCatalog, effective: CoreClass) -> Result<(), SchemaError> {
        self.validate_for(catalog, &super::core::ClassTable::STOCK.row(effective))
    }

    /// Validate using the arena's frozen configurable class row.
    /// # Errors
    /// The same binding errors, under the actual configured profile capacity.
    pub fn validate_for(
        &self,
        catalog: &CardCatalog,
        profile: &super::core::CoreProfile,
    ) -> Result<(), SchemaError> {
        let cap = usize::from(profile.profiles);
        if self.0.is_empty() || self.0.len() > cap {
            return Err(invalid("profiles", "profile count exceeds effective class"));
        }
        let mut seen = BTreeSet::new();
        let definition = self.0.first().map(|entry| entry.key.definition);
        for entry in &self.0 {
            let payload = catalog
                .definition(entry.key.definition)
                .and_then(|d| d.profiles.get(usize::from(entry.key.index)))
                .ok_or_else(|| invalid("profiles", "missing profile payload"))?;
            if definition != Some(entry.key.definition)
                || payload.name != entry.name
                || !seen.insert(entry.key)
            {
                return Err(invalid(
                    "profiles",
                    "mixed, duplicate or mislabeled profile binding",
                ));
            }
            catalog.resolve(entry.key)?;
        }
        Ok(())
    }

    /// Count, bounded by validation.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no binding exists (invalid at admission).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Canonical name at the active zero-based index.
    #[must_use]
    pub fn name(&self, index: u8) -> Option<&str> {
        self.0
            .get(usize::from(index))
            .map(|entry| entry.name.as_str())
    }

    /// Canonical names in sealed order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|entry| entry.name.as_str())
    }

    /// Exact card/program binding at the active zero-based index.
    #[must_use]
    pub fn key(&self, index: u8) -> Option<ProfileKey> {
        self.0.get(usize::from(index)).map(|entry| entry.key)
    }
}

/// Capture every stored Standard field once, including configuration-derived
/// weights and millisecond timers. It never reconstructs from a preset name.
/// The selected cards may be operator-configured values outside the narrower
/// interactive StockCard::set slider grid; no existing configuration is rounded.
///
/// # Errors
/// Empty/oversized sets, unlawful pair, or invalid closed schema values.
pub fn stock_definition(
    selected: &[(SmolStr, StockCard)],
    declared_class: CoreClass,
) -> Result<CardDefinition, SchemaError> {
    let (_, first) = selected
        .first()
        .ok_or_else(|| invalid("profiles", "a stock selection needs an opening card"))?;
    let first_values = stock_values(first)?;
    let mut sections = vec![
        CardSection {
            id: "posture".into(),
            label: "Posture".into(),
            station: super::state::Channel::Helm,
            fields: vec![],
        },
        CardSection {
            id: "fire".into(),
            label: "Fire discipline".into(),
            station: super::state::Channel::Fire,
            fields: vec![],
        },
        CardSection {
            id: "targeting".into(),
            label: "Targeting".into(),
            station: super::state::Channel::Sensors,
            fields: vec![],
        },
        CardSection {
            id: "survival".into(),
            label: "Survival and search".into(),
            station: super::state::Channel::Helm,
            fields: vec![],
        },
    ];
    for (id, value) in &first_values {
        let (section, label) = match id.as_str() {
            "stance" => (0, "Stance"),
            "engage_band_permille" => (0, "Engage band"),
            "close_m" => (0, "Close distance"),
            "orbit_sense" => (0, "Orbit direction"),
            "turn_into_hits" => (0, "Turn into hits"),
            "turn_s" => (0, "Turn duration"),
            "screen_bearing" => (0, "Screen bearing"),
            "screen_arc" => (0, "Screen arc"),
            "opening" => (0, "Opening"),
            "min_odds_permille" => (1, "Minimum odds"),
            "aspect" => (1, "Aspect"),
            "reserve_pct" => (1, "Reserve"),
            "reserve_odds_pct" => (1, "Reserve odds"),
            "prefer" => (2, "Target preference"),
            "weight_range" => (2, "Range weight"),
            "weight_facet" => (2, "Facet weight"),
            "weight_health" => (2, "Health weight"),
            "switch_m" => (2, "Lock switch advantage"),
            "withdraw_permille" => (3, "Break off health"),
            "stand_min_permille" => (3, "Fight home health"),
            "rally_ms" => (3, "Rally delay"),
            "search_ms" => (3, "Search duration"),
            "hunt_after_ms" => (3, "Hunt delay"),
            "stop_unseen" => (3, "Stop unseen"),
            _ => return Err(invalid(id.as_str(), "unknown stored stock field")),
        };
        let help = SmolStr::new_static(
            "Sealed in exact stored units; presets copy values, never behavior.",
        );
        let widget = match value {
            CardValue::Boolean(default) => Widget::Toggle {
                default: *default,
                help,
            },
            CardValue::Word(default) => {
                let choices: &[&str] = match id.as_str() {
                    "stance" => &["hold", "close", "orbit", "hunt", "screen"],
                    "orbit_sense" => &["cw", "ccw", "auto"],
                    "opening" => &["wait", "search", "advance"],
                    "aspect" => &["any", "flank", "rear"],
                    "prefer" => &["nearest", "exposed", "hurt", "balanced", "best_odds"],
                    _ => return Err(invalid(id.as_str(), "unknown stock word")),
                };
                Widget::Choice {
                    choices: choices.iter().map(|word| SmolStr::new(*word)).collect(),
                    default: default.clone(),
                    help,
                }
            }
            CardValue::Integer(default) => {
                let (max, scale, unit) = match id.as_str() {
                    "engage_band_permille"
                    | "withdraw_permille"
                    | "stand_min_permille"
                    | "min_odds_permille" => (1000, 10, "%"),
                    "reserve_pct" | "reserve_odds_pct" => (100, 1, "%"),
                    "screen_bearing" => (359, 1, "degrees"),
                    "screen_arc" => (360, 1, "degrees"),
                    "rally_ms" | "search_ms" | "hunt_after_ms" => (i64::from(u32::MAX), 1000, "s"),
                    "turn_s" => (i64::from(u32::MAX), 1, "s"),
                    "close_m" | "switch_m" => (i64::from(u32::MAX), 1, "m"),
                    _ => (i64::from(u32::MAX), 1, ""),
                };
                Widget::Slider {
                    min: 0,
                    max,
                    step: 1,
                    default: *default,
                    scale,
                    unit: unit.into(),
                    help,
                }
            }
            CardValue::Set(_) => return Err(invalid(id.as_str(), "stock fields are scalar")),
        };
        sections[section].fields.push(CardField {
            id: id.clone(),
            label: label.into(),
            widget,
        });
    }
    let definition = CardDefinition {
        schema: CardSchema { sections },
        declared_class,
        profiles: selected
            .iter()
            .map(|(name, card)| {
                card.pair("profile")
                    .map_err(|error| invalid(name.as_str(), error.to_string()))?;
                Ok(ProfileDefinition {
                    name: name.clone(),
                    values: stock_values(card)?,
                    rules: vec![],
                })
            })
            .collect::<Result<_, SchemaError>>()?,
    };
    definition.validate()?;
    Ok(definition)
}

/// Full stored representation for an explicit stock adapter, with no aliases.
/// # Errors
/// Invalid serialization, including a newly added unsupported stock value type.
pub fn stock_values(card: &StockCard) -> Result<CardValues, SchemaError> {
    serde_json::from_value(json(card)?)
        .map_err(|_| invalid("stock card", "stored fields must be closed scalar values"))
}

/// Server-owned ROM defaults captured once at sealing. This law-derived default
/// never rewrites authored values. Later drops and resume use this exact payload.
pub fn rom_definition(
    cfg: &crate::config::ReflexConfig,
    floor: u32,
) -> Result<CardDefinition, SchemaError> {
    if floor > 1000 {
        return Err(invalid("ROM", "Compact floor exceeds 100%"));
    }
    let mut card = super::card::line(cfg);
    card.min_odds_permille = card
        .min_odds_permille
        .max(u16::try_from(floor).expect("floor checked at most 1000"));
    stock_definition(&[("Line".into(), card)], CoreClass::Zero)
}

fn stock_adapter(values: &CardValues) -> Option<StockCard> {
    // Derive only the serialized field names from the stock type. None of the
    // template's values is used as a fallback: every field must be present in
    // the captured card, with its exact type. Authored extras remain governed by
    // the complete schema validation and stay in the immutable card payload.
    let template =
        stock_values(&super::card::line(&crate::config::ReflexConfig::default())).ok()?;
    let subset = template
        .keys()
        .map(|key| Some((key.clone(), values.get(key)?.clone())))
        .collect::<Option<CardValues>>()?;
    let value = serde_json::to_value(subset).ok()?;
    let card: StockCard = serde_json::from_value(value.clone()).ok()?;
    if serde_json::to_value(&card).ok()? != value || card.pair("card").is_err() {
        return None;
    }
    Some(card)
}

fn definition_size(definition: &CardDefinition) -> Result<usize, SchemaError> {
    // Includes the 64-character JSON key, quotes, colon and entry separator.
    serde_json::to_vec(definition)
        .map(|bytes| bytes.len() + 68)
        .map_err(|_| invalid("catalog", "cannot encode definition"))
}

fn json(value: &impl Serialize) -> Result<serde_json::Value, SchemaError> {
    serde_json::to_value(value).map_err(|_| invalid("card", "cannot encode definition"))
}

fn invalid(field: impl Into<String>, message: impl Into<String>) -> SchemaError {
    SchemaError {
        field: field.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::schema::{CardField, CardSection, Widget};
    use super::super::state::Channel;
    use super::*;

    #[test]
    fn m6_stock_adapter_requires_every_exact_stock_value_and_preserves_authored_extras() {
        let stock = super::super::card::line(&crate::config::ReflexConfig::default());
        let original = stock_values(&stock).expect("stock values");
        let mut extended = original.clone();
        extended.insert("author_flag".into(), CardValue::Boolean(true));
        assert_eq!(stock_adapter(&extended), Some(stock));
        assert_eq!(extended.get("author_flag"), Some(&CardValue::Boolean(true)));
        for key in original.keys() {
            let mut missing = extended.clone();
            missing.remove(key);
            assert!(stock_adapter(&missing).is_none(), "missing {key}");
            let mut wrong_type = extended.clone();
            wrong_type.insert(key.clone(), CardValue::Set(Vec::new()));
            assert!(stock_adapter(&wrong_type).is_none(), "wrong type {key}");
        }
        for value in [CardValue::Word("100".into()), CardValue::Integer(-1)] {
            let mut changed = extended.clone();
            changed.insert("min_odds_permille".into(), value);
            assert!(stock_adapter(&changed).is_none());
        }
        let mut unknown_enum = extended;
        unknown_enum.insert("stance".into(), CardValue::Word("unknown".into()));
        assert!(stock_adapter(&unknown_enum).is_none());
    }

    #[test]
    fn m6_unbound_profiles_are_an_invalid_marker() {
        assert!(
            RuntimeProfiles::unbound()
                .validate(&CardCatalog::default(), CoreClass::I)
                .is_err()
        );
    }

    fn definition(name: &str) -> CardDefinition {
        CardDefinition {
            schema: CardSchema::default(),
            declared_class: CoreClass::I,
            profiles: vec![ProfileDefinition {
                name: name.into(),
                values: BTreeMap::new(),
                rules: vec![],
            }],
        }
    }

    #[test]
    fn m6_cards_three_hundred_hulls_share_one_definition_and_one_runtime_payload() {
        let mut catalog = CardCatalog::default();
        let digest = catalog.admit(definition("Default")).expect("definition");
        let key = ProfileKey {
            definition: digest,
            index: 0,
        };
        let first = catalog.resolve(key).expect("card");
        let mut cards = Vec::new();
        for _ in 0..300 {
            assert_eq!(catalog.admit(definition("Default")).expect("dedup"), digest);
            let card = catalog.resolve(key).expect("same card");
            assert!(Arc::ptr_eq(
                first.data.as_ref().expect("first data"),
                card.data.as_ref().expect("card data")
            ));
            cards.push(card);
        }
        assert_eq!(catalog.len(), 1);
        let encoded = serde_json::to_string(&cards).expect("keys only");
        assert!(!encoded.contains("schema") && !encoded.contains("values"));
        assert!(encoded.len() < 40_000);
    }

    #[test]
    fn m6_cards_snapshot_keys_rehydrate_only_from_valid_catalog() {
        let mut catalog = CardCatalog::default();
        let digest = catalog.admit(definition("Default")).expect("definition");
        let original = catalog
            .resolve(ProfileKey {
                definition: digest,
                index: 0,
            })
            .expect("card");
        let mut card: RuntimeCard =
            serde_json::from_value(serde_json::to_value(&original).expect("encode"))
                .expect("decode key");
        assert!(!card.is_hydrated());
        let mut restored: CardCatalog =
            serde_json::from_value(serde_json::to_value(&catalog).expect("catalog encode"))
                .expect("catalog decode");
        assert!(restored.resolve(card.key()).is_err());
        assert!(restored.admit(definition("Different")).is_err());
        restored.rebuild().expect("validate once");
        restored.hydrate_card(&mut card).expect("exact key");
        assert!(card.is_hydrated());
        assert_eq!(card.canonical_bytes(), original.canonical_bytes());
        assert!(CardCatalog::default().hydrate_card(&mut card).is_err());
        assert!(
            restored
                .resolve(ProfileKey {
                    definition: digest,
                    index: 4
                })
                .is_err()
        );
    }

    #[test]
    fn m6_cards_tampered_definition_is_refused_without_publishing_runtime_data() {
        let mut catalog = CardCatalog::default();
        let digest = catalog.admit(definition("Default")).expect("definition");
        let mut json = serde_json::to_value(&catalog).expect("encode");
        json["definitions"][digest.to_hex()]["profiles"][0]["name"] = serde_json::json!("Tampered");
        let mut restored: CardCatalog = serde_json::from_value(json).expect("wire shape");
        assert!(restored.rebuild().is_err());
        assert!(restored.hydrated.is_empty());
    }

    #[test]
    fn m6_cards_changed_values_change_identity_and_active_hash_input() {
        let mut first = definition("Default");
        first.schema.sections.push(CardSection {
            id: "posture".into(),
            label: "Posture".into(),
            station: Channel::Helm,
            fields: vec![CardField {
                id: "wait".into(),
                label: "Wait".into(),
                widget: Widget::Toggle {
                    default: true,
                    help: "".into(),
                },
            }],
        });
        first.profiles[0]
            .values
            .insert("wait".into(), CardValue::Boolean(true));
        let mut second = first.clone();
        second.profiles[0]
            .values
            .insert("wait".into(), CardValue::Boolean(false));
        let mut catalog = CardCatalog::default();
        let first = catalog.admit(first).expect("first");
        let second = catalog.admit(second).expect("second");
        assert_ne!(first, second);
        let first = catalog
            .resolve(ProfileKey {
                definition: first,
                index: 0,
            })
            .expect("first card");
        let second = catalog
            .resolve(ProfileKey {
                definition: second,
                index: 0,
            })
            .expect("second card");
        assert_ne!(first.key(), second.key());
        assert_ne!(first.canonical_bytes(), second.canonical_bytes());
        assert_eq!(second.get("wait"), Some(&CardValue::Boolean(false)));
        assert!(second.stock().is_none());
    }

    #[test]
    fn m6_cards_named_binding_cannot_reconstruct_a_missing_or_mislabeled_profile() {
        let mut catalog = CardCatalog::default();
        let digest = catalog.admit(definition("Default")).expect("definition");
        let bindings = RuntimeProfiles::bind(&catalog, digest, &["default".into()])
            .expect("case-insensitive selection");
        assert_eq!(bindings.name(0), Some("Default"));
        bindings
            .validate(&catalog, CoreClass::I)
            .expect("complete binding");
        assert!(RuntimeProfiles::bind(&catalog, digest, &["Missing".into()]).is_err());
        let mut forged = serde_json::to_value(&bindings).expect("encode");
        forged[0]["name"] = serde_json::json!("Line");
        let forged: RuntimeProfiles = serde_json::from_value(forged).expect("shape");
        assert!(forged.validate(&catalog, CoreClass::I).is_err());
        let forged = RuntimeProfiles(vec![bindings.0[0].clone(); 2]);
        assert!(forged.validate(&catalog, CoreClass::I).is_err());
    }

    #[test]
    fn m6_cards_catalog_count_bound_refuses_before_mutation() {
        let mut catalog = CardCatalog::default();
        for index in 0..MAX_CARD_DEFINITIONS {
            catalog
                .admit(definition(&format!("P{index}")))
                .expect("within count");
        }
        let before = serde_json::to_vec(&catalog).expect("before");
        assert!(catalog.admit(definition("Overflow")).is_err());
        assert_eq!(serde_json::to_vec(&catalog).expect("after"), before);
    }

    #[test]
    fn m6_cards_aggregate_byte_bound_is_separate_from_each_valid_definition() {
        let mut large = definition("Default");
        let fields = (0..64)
            .map(|index| CardField {
                id: format!("f{index}").into(),
                label: "Parameter".into(),
                widget: Widget::Toggle {
                    default: false,
                    help: "x".repeat(512).into(),
                },
            })
            .collect();
        large.schema.sections.push(CardSection {
            id: "settings".into(),
            label: "Settings".into(),
            station: Channel::Helm,
            fields,
        });
        large.profiles[0].values = large.schema.defaults();
        large.validate().expect("individually valid");
        let mut catalog = CardCatalog::default();
        for index in 0..MAX_CARD_DEFINITIONS {
            large.profiles[0].name = format!("P{index}").into();
            let before = catalog.len();
            let bytes = catalog.encoded_bytes();
            if catalog.admit(large.clone()).is_err() {
                assert_eq!(catalog.len(), before);
                assert_eq!(catalog.encoded_bytes(), bytes);
                assert!(before < MAX_CARD_DEFINITIONS);
                assert!(bytes <= MAX_CARD_CATALOG_BYTES);
                return;
            }
        }
        panic!("aggregate bytes should refuse before the count cap");
    }
    #[test]
    fn m6_stock_all_24_values_survive_catalog_round_trip_without_config() {
        let cfg = crate::config::ReflexConfig {
            weight_range: u32::MAX,
            weight_facet: 123,
            weight_health: 456,
            engage_band_permille: 711,
            rally_ms: 20_013,
            search_ms: 30_017,
            hunt_after_ms: u32::MAX,
            ..crate::config::ReflexConfig::default()
        };
        let line = super::super::card::line(&cfg);
        let mut alternate = line.clone();
        alternate.close_m = 350;
        let definition = stock_definition(
            &[
                ("Line".into(), line.clone()),
                ("Alternate".into(), alternate.clone()),
            ],
            CoreClass::I,
        )
        .unwrap();
        assert_eq!(definition.profiles[0].values.len(), 24);
        let mut catalog = CardCatalog::default();
        let digest = catalog.admit(definition).unwrap();
        let encoded = serde_json::to_vec(&catalog).unwrap();
        let mut decoded: CardCatalog = serde_json::from_slice(&encoded).unwrap();
        decoded.rebuild().unwrap();
        assert_eq!(
            decoded
                .resolve(ProfileKey {
                    definition: digest,
                    index: 0
                })
                .unwrap()
                .stock(),
            Some(&line)
        );
        assert_eq!(
            decoded
                .resolve(ProfileKey {
                    definition: digest,
                    index: 1
                })
                .unwrap()
                .stock(),
            Some(&alternate)
        );
        assert_ne!(
            line,
            super::super::card::line(&crate::config::ReflexConfig::default())
        );
    }
}
