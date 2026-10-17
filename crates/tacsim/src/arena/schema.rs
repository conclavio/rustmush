//! Pure, bounded schema, card and rule envelopes for authored minds.
//!
//! This is the single semantic decoder for save, check, UI and sealing. Numbers
//! are integers; a slider's scale supplies decimal display without rounding or
//! floating point. Full profile values are immutable executable data. A preset
//! copies values into a profile; it never carries hidden behavior.
//!
//! Stations describe controls, not a required chassis capability. A station
//! absent on a hull makes its section inert; it does not invalidate this schema.
//! Source compilation, ABI, manifest checks and concrete chassis/floor admission
//! belong to the composition boundary. This module reads no live arena facts.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use smol_str::SmolStr;

use super::conditions::{ASPECTS_ANY, Rule, RulePredicate, SlotTarget, validate_rule_shape};
use super::core::{ClassTable, CoreClass};
use super::state::Channel;

/// Maximum bytes of executable metadata, independently of the source limit.
pub const MAX_SCHEMA_BYTES: usize = 64 * 1024;
/// Maximum sections in the closed widget schema.
pub const MAX_SECTIONS: usize = 16;
/// Maximum named fields over every section.
pub const MAX_FIELDS: usize = 64;
/// Maximum options in a choice widget.
pub const MAX_CHOICES: usize = 32;
/// Maximum duration of a recent-hit predicate, in arena milliseconds.
pub const MAX_HIT_WITHIN_MS: u32 = 300_000;

/// A safe refusal identifying the schema/card field that failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{field}: {message}")]
pub struct SchemaError {
    /// Path within the executable envelope.
    pub field: String,
    /// Bounded diagnostic, never source text.
    pub message: String,
}

fn invalid(field: impl Into<String>, message: impl Into<String>) -> SchemaError {
    SchemaError {
        field: field.into(),
        message: message.into(),
    }
}

/// Ordered station sections; an empty schema is valid for a parameter-free mind.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardSchema {
    /// Presentation order; field identifiers are unique across all sections.
    pub sections: Vec<CardSection>,
}

/// One labelled station section. Missing hardware is explained by the renderer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardSection {
    /// Stable lowercase identifier.
    pub id: SmolStr,
    /// User-facing label.
    pub label: SmolStr,
    /// Existing station word: helm, sensors or fire.
    pub station: Channel,
    /// Controls in presentation order.
    pub fields: Vec<CardField>,
}

/// One stable field and its closed widget definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardField {
    /// Globally unique lowercase identifier used by the script accessor.
    pub id: SmolStr,
    /// User-facing label.
    pub label: SmolStr,
    /// Widget and its complete validation contract.
    pub widget: Widget,
}

/// The closed widget vocabulary. Bounds/defaults use exactly the card's units.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Widget {
    /// Integer ticks displayed as ticks/scale; half-percent uses scale 10, step 5.
    Slider {
        /// Inclusive minimum tick value.
        min: i64,
        /// Inclusive maximum tick value.
        max: i64,
        /// Positive tick spacing anchored at min.
        step: i64,
        /// Valid default tick value.
        default: i64,
        /// Decimal display divisor, one of 1, 10, 100 or 1000.
        scale: u16,
        /// Short display unit, such as %, m, s or ms.
        unit: SmolStr,
        /// Bounded explanation.
        help: SmolStr,
    },
    /// A closed, ordered collection of distinct lowercase words.
    Choice {
        /// Allowed option identifiers.
        choices: Vec<SmolStr>,
        /// One of choices.
        default: SmolStr,
        /// Bounded explanation.
        help: SmolStr,
    },
    /// A boolean switch.
    Toggle {
        /// Initial boolean.
        default: bool,
        /// Bounded explanation.
        help: SmolStr,
    },
    /// Whole compass degrees, 0 through 359, with no wrapping or clamp.
    Bearing {
        /// Initial whole degrees.
        default: u16,
        /// Bounded explanation.
        help: SmolStr,
    },
    /// A canonical sorted subset of front, left, rear, right and top.
    FacetSet {
        /// Initial set, sorted and without duplicates.
        default: Vec<SmolStr>,
        /// Bounded explanation.
        help: SmolStr,
    },
    /// Relative earned classification, independent of scenario team names.
    /// Unknown has no earned side; friendly has the caller's side; hostile has
    /// an earned other side. A class selection does not reveal hidden contacts.
    ContactClassSet {
        /// Sorted subset of friendly, hostile and unknown.
        default: Vec<SmolStr>,
        /// Bounded explanation.
        help: SmolStr,
    },
}

/// A field value. Sets are canonical sorted arrays, never silently normalized.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CardValue {
    /// Integer ticks, including bearing degrees.
    Integer(i64),
    /// Toggle.
    Boolean(bool),
    /// Choice identifier.
    Word(SmolStr),
    /// Sorted closed-vocabulary set.
    Set(Vec<SmolStr>),
}

/// Complete values, keyed by stable field identifier.
pub type CardValues = BTreeMap<SmolStr, CardValue>;
/// Named complete cards. Names preserve their case; lookup uses the exact name.
pub type NamedCards = BTreeMap<SmolStr, CardValues>;

/// Standing-order kind, never an object id or an unearned contact reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderKind {
    /// No live standing order.
    None,
    /// Hold.
    Hold,
    /// Advance.
    Advance,
    /// Engage.
    Engage,
    /// Withdraw.
    Withdraw,
    /// Screen.
    Screen,
    /// Support.
    Support,
}

/// Conditions which may run only as part of a budgeted think over its public view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "predicate", rename_all = "snake_case", deny_unknown_fields)]
pub enum ThinkPredicate {
    /// A public own-hit event no older than the given arena duration.
    HitWithin {
        /// Positive duration, at most MAX_HIT_WITHIN_MS.
        milliseconds: u32,
    },
    /// Classified living hostiles in the bounded view, never hidden arena count.
    Hostiles {
        /// Inclusive minimum visible hostile count.
        at_least: u8,
    },
    /// The public live standing-order kind.
    Order {
        /// Desired kind.
        is: OrderKind,
    },
}

/// A native action and its AND terms, with optional think-only gates.
///
/// Pure native rules can be installed on the event schedule. Any think terms
/// make the whole rule a think program: they must never be discarded and leave
/// an accidentally unconditional native action behind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredRule {
    /// Shared native action/predicates. Internal placement flags must be false.
    pub rule: Rule,
    /// Extra conditions; total predicate count over both lists is at most four.
    pub think: Vec<ThinkPredicate>,
}

/// Render an editable command in the shared author grammar, preserving term
/// order and symbolic ALL. This is separate from the English display above.
/// No source location, contact identity or inferred predicate is introduced.
///
/// # Errors
/// Structured-only forms without an exact textual spelling are refused; callers
/// retain the original draft and display rather than rewriting its semantics.
pub fn rule_source(authored: &AuthoredRule) -> Result<String, SchemaError> {
    use super::conditions::{Comparison, LockSelector, RuleBand, SectionState, TargetState};
    authored.validate()?;
    let unsupported = || {
        invalid(
            "rule",
            "This structured rule has no exact editable text spelling.",
        )
    };
    let symbol = |value: &str| -> Result<String, SchemaError> {
        if value.bytes().any(|b| b.is_ascii_uppercase()) {
            Err(unsupported())
        } else {
            Ok(value.to_owned())
        }
    };
    let compare = |comparison: Comparison| match comparison {
        Comparison::Less => Ok("<"),
        Comparison::Greater => Ok(">"),
        Comparison::AtLeast | Comparison::Equal => Err(unsupported()),
    };
    let mut terms = Vec::new();
    let action = match &authored.rule {
        Rule::Fire { condition, .. } => {
            if condition.target == SlotTarget::BestHostile {
                terms.push("target=best_hostile".to_owned());
            }
            format!("FIRE {}", symbol(&condition.weapon)?)
        }
        Rule::HoldFire { weapon, .. } => format!("FIRE {} HOLD", symbol(weapon)?),
        Rule::Lock {
            selector, weapon, ..
        } => {
            let selected =
                authored
                    .rule
                    .predicates()
                    .iter()
                    .find_map(|predicate| match predicate {
                        RulePredicate::Odds { weapon, .. } => weapon.clone(),
                        _ => None,
                    });
            if *weapon != selected {
                return Err(unsupported());
            }
            format!(
                "LOCK {}",
                match selector {
                    LockSelector::Nearest => "nearest",
                    LockSelector::BestOdds => "best_odds",
                    LockSelector::Weakest => "weakest",
                    LockSelector::Flank => "flank",
                    LockSelector::Newest => "newest",
                }
            )
        }
    };
    for (index, predicate) in authored.rule.predicates().iter().enumerate() {
        // The text parser deduplicates identical terms. Do not erase one from a
        // structured draft while presenting the result as an exact edit.
        if authored
            .rule
            .predicates()
            .get(..index)
            .is_some_and(|prior| prior.contains(predicate))
        {
            return Err(unsupported());
        }
        let term = match predicate {
            RulePredicate::Odds {
                weapon,
                min_permille,
            } => {
                if *min_permille == 0 || min_permille % 10 != 0 {
                    return Err(unsupported());
                }
                let operand = weapon
                    .as_deref()
                    .map(&symbol)
                    .transpose()?
                    .map(|w| format!(" {w}"))
                    .unwrap_or_default();
                format!("odds{operand}>={}", min_permille / 10)
            }
            RulePredicate::Aspect { mask } => {
                if mask & 0b0110 != 0 && mask & 0b0110 != 0b0110 {
                    return Err(unsupported());
                }
                let names = [(0b0001, "front"), (0b0110, "side"), (0b1000, "rear")]
                    .into_iter()
                    .filter_map(|(bits, name)| (mask & bits != 0).then_some(name))
                    .collect::<Vec<_>>();
                format!("aspect={}", names.join(","))
            }
            RulePredicate::Range { comparison, metres } => {
                if !(1..=super::rules::MAX_RULE_RANGE_M).contains(metres) {
                    return Err(unsupported());
                }
                format!("range{}{}m", compare(*comparison)?, metres)
            }
            RulePredicate::Band { band } => format!(
                "band={}",
                match band {
                    RuleBand::Close => "close",
                    RuleBand::Effective => "effective",
                    RuleBand::Far => "far",
                }
            ),
            RulePredicate::Target { state } => format!(
                "target={}",
                match state {
                    TargetState::Identified => "identified",
                    TargetState::Moving => "moving",
                    TargetState::Stopped => "stopped",
                    TargetState::Hurt => "hurt",
                }
            ),
            RulePredicate::TargetSection { section, state } => format!(
                "target {}={}",
                symbol(section)?,
                match state {
                    SectionState::Green => "green",
                    SectionState::Yellow => "yellow",
                    SectionState::Red => "red",
                    SectionState::Destroyed => "destroyed",
                }
            ),
            RulePredicate::Rounds {
                weapon,
                comparison,
                amount,
                percent,
            } => {
                let operand = weapon
                    .as_deref()
                    .map(&symbol)
                    .transpose()?
                    .map(|w| format!(" {w}"))
                    .unwrap_or_default();
                format!(
                    "rounds{operand}{}{}{}",
                    compare(*comparison)?,
                    amount,
                    if *percent { "%" } else { "" }
                )
            }
            RulePredicate::OwnSection {
                section,
                below_percent,
            } => format!("own {}<{}%", symbol(section)?, below_percent),
            RulePredicate::Heat { above_percent } => format!("heat>{above_percent}%"),
            RulePredicate::Lock { present } => if *present { "lock" } else { "no lock" }.to_owned(),
        };
        terms.push(term);
    }
    for term in &authored.think {
        terms.push(match term {
            ThinkPredicate::HitWithin { milliseconds } => format!("hit within {milliseconds}ms"),
            ThinkPredicate::Hostiles { at_least } => format!("hostiles>={at_least}"),
            ThinkPredicate::Order { is } => format!(
                "order={}",
                match is {
                    OrderKind::None => "none",
                    OrderKind::Hold => "hold",
                    OrderKind::Advance => "advance",
                    OrderKind::Engage => "engage",
                    OrderKind::Withdraw => "withdraw",
                    OrderKind::Screen => "screen",
                    OrderKind::Support => "support",
                }
            ),
        });
    }
    if terms.is_empty() {
        if matches!(authored.rule, Rule::Fire { .. }) {
            // An explicit default selector adds no predicate or hidden floor.
            terms.push("target=lock".to_owned());
        } else {
            return Err(unsupported());
        }
    }
    let source = format!("{action} WHEN {}", terms.join(" AND "));
    if source.len() > super::rules::MAX_RULE_BYTES {
        return Err(unsupported());
    }
    Ok(source)
}

/// Ordered profile rules; absent profile entries mean no rules.
pub type ProfileRules = BTreeMap<SmolStr, Vec<AuthoredRule>>;

/// The native action and native terms keep their canonical English renderer;
/// think terms append to that same conjunction without inventing another action.
pub fn rule_words(authored: &AuthoredRule) -> String {
    let mut text = crate::arena::rules::rule_words(&authored.rule, None);
    for term in &authored.think {
        let clause = match term {
            ThinkPredicate::HitWithin { milliseconds } => {
                format!("an own hit was reported within {milliseconds} ms")
            }
            ThinkPredicate::Hostiles { at_least } => {
                format!("at least {at_least} classified living hostiles are on the plot")
            }
            ThinkPredicate::Order { is } => format!(
                "your standing order is {}",
                match is {
                    OrderKind::None => "none",
                    OrderKind::Hold => "hold",
                    OrderKind::Advance => "advance",
                    OrderKind::Engage => "engage",
                    OrderKind::Withdraw => "withdraw",
                    OrderKind::Screen => "screen",
                    OrderKind::Support => "support",
                }
            ),
        };
        text.push_str(if text.contains(" when ") {
            " and "
        } else {
            " when "
        });
        text.push_str(&clause);
    }
    text
}

/// Fully decoded executable metadata. Construct through validate_envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedEnvelope {
    /// Closed schema.
    pub schema: CardSchema,
    /// Reusable named cards.
    pub presets: NamedCards,
    /// Complete sealed-profile candidates.
    pub profiles: NamedCards,
    /// Ordered programs per profile.
    pub rules: ProfileRules,
    /// The declaration used for format-level capacity checks.
    pub declared_class: CoreClass,
}

/// Decode and validate all four metadata objects without source or world access.
///
/// Core zero is valid for the ROM's schema. The authored-source adapter must
/// separately refuse a script claiming class zero. Effective class, configured
/// class-table limits, station authority, weapon expansion and current Compact
/// floor are rechecked at selection/check-out. No field is clamped or defaulted.
///
/// # Errors
/// A named refusal for unknown fields, shapes, vocabulary, values or capacities.
pub fn validate_envelope(
    schema: &Value,
    presets: &Value,
    profiles: &Value,
    rules: &Value,
    declared_class: CoreClass,
) -> Result<ValidatedEnvelope, SchemaError> {
    let values = [schema, presets, profiles, rules];
    let mut nodes = 0_usize;
    let mut text_bytes = 0_usize;
    for value in values {
        bounded_json(value, 0, &mut nodes, &mut text_bytes)?;
    }
    let bytes =
        serde_json::to_vec(&values).map_err(|_| invalid("envelope", "cannot encode metadata"))?;
    if bytes.len() > MAX_SCHEMA_BYTES {
        return Err(invalid("envelope", "metadata exceeds 65536 bytes"));
    }
    let result = ValidatedEnvelope {
        schema: decode(schema, "schema")?,
        presets: decode(presets, "presets")?,
        profiles: decode(profiles, "profiles")?,
        rules: decode(rules, "rules")?,
        declared_class,
    };
    result.validate()?;
    Ok(result)
}

fn decode<T: DeserializeOwned + Serialize>(value: &Value, field: &str) -> Result<T, SchemaError> {
    let decoded: T = serde_json::from_value(value.clone()).map_err(|error| {
        invalid(
            field,
            error.to_string().chars().take(240).collect::<String>(),
        )
    })?;
    // Also protects reused nested types which do not themselves deny unknown
    // fields, and rejects omitted/null alternatives to the canonical encoding.
    let encoded = serde_json::to_value(&decoded)
        .map_err(|_| invalid(field, "cannot encode validated metadata"))?;
    if &encoded != value {
        return Err(invalid(
            field,
            "use the complete canonical shape; unknown or omitted fields are refused",
        ));
    }
    Ok(decoded)
}

impl ValidatedEnvelope {
    fn validate(&self) -> Result<(), SchemaError> {
        self.schema.validate()?;
        let caps = ClassTable::STOCK.row(self.declared_class);
        if self.profiles.is_empty() || self.profiles.len() > usize::from(caps.profiles) {
            return Err(invalid(
                "profiles",
                format!(
                    "class {} carries 1 through {} profiles",
                    self.declared_class.as_str(),
                    caps.profiles
                ),
            ));
        }
        if self.presets.len() > 8 {
            return Err(invalid("presets", "at most eight presets"));
        }
        for (kind, cards) in [("presets", &self.presets), ("profiles", &self.profiles)] {
            let mut names = BTreeSet::new();
            for (name, values) in cards {
                name_word(name, &format!("{kind}.{name}"))?;
                if !names.insert(name.to_ascii_lowercase()) {
                    return Err(invalid(kind, "names must differ without regard to case"));
                }
                self.schema
                    .validate_values(values, &format!("{kind}.{name}"))?;
            }
        }
        for (profile, rules) in &self.rules {
            let path = format!("rules.{profile}");
            if !self.profiles.contains_key(profile) {
                return Err(invalid(&path, "no profile has this exact name"));
            }
            if rules.len() > caps.slots {
                return Err(invalid(
                    &path,
                    format!(
                        "class {} carries at most {} rules",
                        self.declared_class.as_str(),
                        caps.slots
                    ),
                ));
            }
            for (index, rule) in rules.iter().enumerate() {
                rule.validate_at(&format!("{path}[{index}]"))?;
            }
        }
        Ok(())
    }

    /// Refuse explicit odds below the captured floor; sealing calls this again.
    ///
    /// # Errors
    /// A named rule refusal, or an invalid floor outside 0 through 1000.
    pub fn validate_floor(&self, floor_permille: u16) -> Result<(), SchemaError> {
        if floor_permille > 1000 {
            return Err(invalid("floor", "expected per mille from 0 through 1000"));
        }
        for (profile, rules) in &self.rules {
            for (index, authored) in rules.iter().enumerate() {
                if authored
                    .rule
                    .fire()
                    .is_some_and(|fire| fire.min_permille < floor_permille)
                    || authored.rule.predicates().iter().any(|predicate| {
                        matches!(predicate,
                        RulePredicate::Odds { min_permille, .. } if *min_permille < floor_permille)
                    })
                {
                    return Err(invalid(
                        format!("rules.{profile}[{index}]"),
                        format!("odds below the Compact floor of {floor_permille} per mille"),
                    ));
                }
                if let Some(fire) = authored.rule.fire() {
                    let expected = authored
                        .rule
                        .predicates()
                        .iter()
                        .filter_map(|predicate| match predicate {
                            RulePredicate::Odds {
                                weapon,
                                min_permille,
                            } if weapon.as_ref().is_none_or(|weapon| *weapon == fire.weapon) => {
                                Some(*min_permille)
                            }
                            _ => None,
                        })
                        .max()
                        .unwrap_or(floor_permille)
                        .max(floor_permille);
                    if fire.min_permille != expected {
                        return Err(invalid(
                            format!("rules.{profile}[{index}]"),
                            "fire minimum must equal its explicit weapon odds or the current floor; no hidden fifth term",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

impl CardSchema {
    /// Validate schema declarations without requiring any installed station.
    ///
    /// # Errors
    /// A named declaration or bound which cannot be rendered or validated.
    pub fn validate(&self) -> Result<(), SchemaError> {
        if self.sections.len() > MAX_SECTIONS {
            return Err(invalid("schema.sections", "at most sixteen sections"));
        }
        let mut sections = BTreeSet::new();
        let mut fields = BTreeSet::new();
        for section in &self.sections {
            let path = format!("schema.{}", section.id);
            identifier(&section.id, &path)?;
            text(&section.label, &path, 96, false)?;
            if !sections.insert(&section.id) {
                return Err(invalid(&path, "duplicate section identifier"));
            }
            for field in &section.fields {
                let path = format!("{path}.{}", field.id);
                identifier(&field.id, &path)?;
                text(&field.label, &path, 96, false)?;
                if !fields.insert(&field.id) {
                    return Err(invalid(&path, "field identifiers must be globally unique"));
                }
                if fields.len() > MAX_FIELDS {
                    return Err(invalid("schema", "at most 64 fields across all sections"));
                }
                field.widget.validate(&path)?;
            }
        }
        Ok(())
    }

    /// Return complete default values; callers first validate the schema.
    #[must_use]
    pub fn defaults(&self) -> CardValues {
        self.sections
            .iter()
            .flat_map(|section| &section.fields)
            .map(|field| (field.id.clone(), field.widget.default_value()))
            .collect()
    }

    /// Validate exact values, refusing both undeclared keys and missing keys.
    ///
    /// # Errors
    /// The missing, undeclared or invalid field is named; nothing is clamped.
    pub fn validate_values(&self, values: &CardValues, path: &str) -> Result<(), SchemaError> {
        let fields: BTreeMap<_, _> = self
            .sections
            .iter()
            .flat_map(|section| &section.fields)
            .map(|field| (&field.id, &field.widget))
            .collect();
        for name in values.keys() {
            if !fields.contains_key(name) {
                return Err(invalid(
                    format!("{path}.{name}"),
                    "field is not declared by this schema",
                ));
            }
        }
        for (name, widget) in fields {
            let field_path = format!("{path}.{name}");
            let value = values
                .get(name)
                .ok_or_else(|| invalid(&field_path, "value is missing"))?;
            widget.validate_value(value, &field_path)?;
        }
        Ok(())
    }
}

impl Widget {
    fn validate(&self, path: &str) -> Result<(), SchemaError> {
        let help = match self {
            Self::Slider {
                min,
                max,
                step,
                scale,
                unit,
                help,
                ..
            } => {
                if *min < -4_294_967_295
                    || *max > 4_294_967_295
                    || min > max
                    || *step <= 0
                    || *step > 4_294_967_295
                {
                    return Err(invalid(
                        path,
                        "slider needs ordered bounds within +/-1000000000 and a positive step",
                    ));
                }
                if !matches!(scale, 1 | 10 | 100 | 1000) {
                    return Err(invalid(path, "scale must be 1, 10, 100 or 1000"));
                }
                if (max - min) % step != 0 {
                    return Err(invalid(path, "slider maximum must lie on its step grid"));
                }
                text(unit, path, 24, true)?;
                help
            }
            Self::Choice { choices, help, .. } => {
                if choices.is_empty() || choices.len() > MAX_CHOICES {
                    return Err(invalid(path, "choice needs 1 through 32 options"));
                }
                let mut seen = BTreeSet::new();
                for choice in choices {
                    identifier(choice, path)?;
                    if !seen.insert(choice) {
                        return Err(invalid(path, "choice options must be distinct"));
                    }
                }
                help
            }
            Self::Toggle { help, .. }
            | Self::Bearing { help, .. }
            | Self::FacetSet { help, .. }
            | Self::ContactClassSet { help, .. } => help,
        };
        text(help, path, 512, true)?;
        self.validate_value(&self.default_value(), &format!("{path}.default"))
    }

    /// Complete default in the same representation a script reads.
    #[must_use]
    pub fn default_value(&self) -> CardValue {
        match self {
            Self::Slider { default, .. } => CardValue::Integer(*default),
            Self::Choice { default, .. } => CardValue::Word(default.clone()),
            Self::Toggle { default, .. } => CardValue::Boolean(*default),
            Self::Bearing { default, .. } => CardValue::Integer(i64::from(*default)),
            Self::FacetSet { default, .. } | Self::ContactClassSet { default, .. } => {
                CardValue::Set(default.clone())
            }
        }
    }

    /// Refuse a value outside the exact widget domain; no implicit conversion.
    ///
    /// # Errors
    /// The field and permitted range/type are named.
    pub fn validate_value(&self, value: &CardValue, path: &str) -> Result<(), SchemaError> {
        let valid = match (self, value) {
            (Self::Slider { min, max, step, .. }, CardValue::Integer(value)) => {
                *step > 0
                    && value >= min
                    && value <= max
                    && (i128::from(*value) - i128::from(*min)) % i128::from(*step) == 0
            }
            (Self::Choice { choices, .. }, CardValue::Word(value)) => choices.contains(value),
            (Self::Toggle { .. }, CardValue::Boolean(_)) => true,
            (Self::Bearing { .. }, CardValue::Integer(value)) => (0..=359).contains(value),
            (Self::FacetSet { .. }, CardValue::Set(values)) => {
                set_values(values, &["front", "left", "rear", "right", "top"])
            }
            (Self::ContactClassSet { .. }, CardValue::Set(values)) => {
                set_values(values, &["friendly", "hostile", "unknown"])
            }
            _ => false,
        };
        if valid {
            return Ok(());
        }
        let message = match self {
            Self::Slider {
                min,
                max,
                step,
                scale,
                ..
            } => format!(
                "expected integer ticks {min} through {max}, step {step} from {min}, display scale {scale}"
            ),
            Self::Choice { .. } => "expected one declared choice".to_owned(),
            Self::Toggle { .. } => "expected true or false".to_owned(),
            Self::Bearing { .. } => "expected whole degrees 0 through 359".to_owned(),
            Self::FacetSet { .. } => {
                "expected a sorted unique subset of front, left, rear, right, top".to_owned()
            }
            Self::ContactClassSet { .. } => {
                "expected a sorted unique subset of friendly, hostile, unknown".to_owned()
            }
        };
        Err(invalid(path, message))
    }
}

impl AuthoredRule {
    /// Validate one complete authored row before save or chassis binding.
    /// # Errors
    /// Invalid shape, hidden legacy constraints, internal flags or term bounds.
    pub fn validate(&self) -> Result<(), SchemaError> {
        self.validate_at("rule")
    }

    fn validate_at(&self, path: &str) -> Result<(), SchemaError> {
        validate_rule_shape(&self.rule).map_err(|_| invalid(path, "invalid native rule shape"))?;
        let target_term = usize::from(
            self.rule
                .fire()
                .is_some_and(|fire| fire.target == SlotTarget::BestHostile),
        );
        if self.rule.predicates().len() + self.think.len() + target_term > 4 {
            return Err(invalid(
                path,
                "at most four AND predicates across native and think terms",
            ));
        }
        let internal_flags = match &self.rule {
            Rule::Fire {
                implicit,
                independent,
                ..
            } => *implicit || *independent,
            Rule::HoldFire { implicit, .. } | Rule::Lock { implicit, .. } => *implicit,
        };
        if internal_flags {
            return Err(invalid(
                path,
                "authored rules cannot set internal placement or independent-gun flags",
            ));
        }
        if let Some(fire) = self.rule.fire() {
            if matches!(fire.target, SlotTarget::Contact { .. }) {
                return Err(invalid(
                    path,
                    "a saved rule cannot retain a sortie contact id",
                ));
            }
            // New authored grammar expresses these constraints in predicates.
            // Legacy shorthand's compiled bounds must not smuggle extra terms.
            if fire.aspects != ASPECTS_ANY
                || fire.min_rounds_left != 0
                || fire.min_range_m != 0
                || fire.max_range_m != u32::MAX
            {
                return Err(invalid(
                    path,
                    "use predicates for aspect, rounds and range; legacy compiled bounds are not authored terms",
                ));
            }
        }
        let mut kinds = BTreeSet::new();
        for predicate in &self.think {
            let kind = match predicate {
                ThinkPredicate::HitWithin { milliseconds } => {
                    if *milliseconds == 0 || *milliseconds > MAX_HIT_WITHIN_MS {
                        return Err(invalid(
                            path,
                            "hit_within milliseconds must be 1 through 300000",
                        ));
                    }
                    0
                }
                ThinkPredicate::Hostiles { at_least } => {
                    if *at_least == 0 || *at_least > 24 {
                        return Err(invalid(
                            path,
                            "hostiles at_least must be 1 through 24 visible rows",
                        ));
                    }
                    1
                }
                ThinkPredicate::Order { .. } => 2,
            };
            if !kinds.insert(kind) {
                return Err(invalid(
                    path,
                    "one predicate of each think-only kind is allowed",
                ));
            }
        }
        Ok(())
    }
}

fn set_values(values: &[SmolStr], choices: &[&str]) -> bool {
    values.len() <= choices.len()
        && values.iter().all(|value| choices.contains(&value.as_str()))
        && values.windows(2).all(|pair| pair.first() < pair.get(1))
}

fn identifier(value: &str, path: &str) -> Result<(), SchemaError> {
    if value.is_empty()
        || value.len() > 64
        || !value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(invalid(
            path,
            "expected a lowercase identifier, at most 64 bytes",
        ));
    }
    Ok(())
}

fn name_word(value: &str, path: &str) -> Result<(), SchemaError> {
    if value.is_empty()
        || value.len() > 64
        || !value.is_ascii()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(invalid(
            path,
            "expected a name of 1 through 64 ASCII letters, digits, underscores or hyphens",
        ));
    }
    Ok(())
}

fn text(value: &str, path: &str, max: usize, empty: bool) -> Result<(), SchemaError> {
    if (!empty && value.is_empty()) || value.len() > max || value.chars().any(char::is_control) {
        return Err(invalid(
            path,
            format!("text must be at most {max} bytes without control characters"),
        ));
    }
    Ok(())
}

fn bounded_json(
    value: &Value,
    depth: usize,
    nodes: &mut usize,
    bytes: &mut usize,
) -> Result<(), SchemaError> {
    *nodes += 1;
    if depth > 16 || *nodes > 4096 {
        return Err(invalid(
            "envelope",
            "metadata exceeds depth 16 or 4096 nodes",
        ));
    }
    match value {
        Value::Null | Value::Bool(_) => {}
        Value::Number(number)
            if number.as_i64().is_some_and(|value| {
                (-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&value)
            }) => {}
        Value::Number(_) => {
            return Err(invalid(
                "envelope",
                "metadata numbers must be exactly representable integers (absolute value at most 9007199254740991)",
            ));
        }
        Value::String(value) => {
            if value.len() > 8192 {
                return Err(invalid("envelope", "metadata string exceeds 8192 bytes"));
            }
            *bytes += value.len();
        }
        Value::Array(values) => {
            for value in values {
                bounded_json(value, depth + 1, nodes, bytes)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if key.len() > 128 {
                    return Err(invalid("envelope", "metadata key exceeds 128 bytes"));
                }
                *bytes += key.len();
                bounded_json(value, depth + 1, nodes, bytes)?;
            }
        }
    }
    if *bytes > MAX_SCHEMA_BYTES {
        return Err(invalid("envelope", "metadata exceeds 65536 text bytes"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::conditions::{FireCondition, LockSelector};
    use super::*;
    use serde_json::json;

    fn empty() -> [Value; 4] {
        [
            json!({"sections":[]}),
            json!({}),
            json!({"Default":{}}),
            json!({}),
        ]
    }

    fn validate(values: &[Value; 4], class: CoreClass) -> Result<ValidatedEnvelope, SchemaError> {
        let [schema, presets, profiles, rules] = values;
        validate_envelope(schema, presets, profiles, rules, class)
    }

    fn widgets() -> [Value; 4] {
        let schema = json!({"sections":[{"id":"posture","label":"Posture","station":"helm","fields":[
            {"id":"stand","label":"Fight home","widget":{"kind":"slider","min":0,"max":1000,"step":5,"default":175,"scale":10,"unit":"%","help":"Half-percent steps."}},
            {"id":"stance","label":"Stance","widget":{"kind":"choice","choices":["hold","close"],"default":"hold","help":""}},
            {"id":"wait","label":"Wait","widget":{"kind":"toggle","default":true,"help":""}},
            {"id":"bearing","label":"Bearing","widget":{"kind":"bearing","default":359,"help":""}},
            {"id":"facets","label":"Facets","widget":{"kind":"facet_set","default":["front","top"],"help":""}},
            {"id":"classes","label":"Classes","widget":{"kind":"contact_class_set","default":["hostile","unknown"],"help":""}}
        ]}]});
        let values = json!({"stand":175,"stance":"hold","wait":true,"bearing":359,"facets":["front","top"],"classes":["hostile","unknown"]});
        [
            schema,
            json!({"Line":values.clone()}),
            json!({"Default":values}),
            json!({}),
        ]
    }

    fn fire() -> AuthoredRule {
        AuthoredRule {
            rule: Rule::fire_rule(
                FireCondition {
                    weapon: "cannon".into(),
                    target: SlotTarget::Lock,
                    min_permille: 100,
                    aspects: ASPECTS_ANY,
                    min_rounds_left: 0,
                    min_range_m: 0,
                    max_range_m: u32::MAX,
                },
                Vec::new(),
            ),
            think: Vec::new(),
        }
    }

    fn with_rules(rules: Vec<AuthoredRule>) -> [Value; 4] {
        let mut values = empty();
        values[3] = json!({"Default":rules});
        values
    }

    #[test]
    fn m6_schema_parameter_free_and_rom_cards_have_no_forced_widgets() {
        for class in CoreClass::ALL {
            let result = validate(&empty(), class).expect("empty schema and full empty profile");
            assert!(result.schema.defaults().is_empty());
            result.validate_floor(100).expect("no fire terms");
        }
    }

    #[test]
    fn m6_schema_closed_widgets_keep_half_percent_ticks_and_top_facets() {
        let result = validate(&widgets(), CoreClass::I).expect("six widget kinds");
        assert_eq!(result.schema.defaults(), result.profiles["Default"]);
        assert_eq!(result.profiles["Default"]["stand"], CardValue::Integer(175));
        assert_eq!(
            result.profiles["Default"]["facets"],
            CardValue::Set(vec!["front".into(), "top".into()])
        );
    }

    #[test]
    fn m6_schema_refuses_values_without_clamping_or_coercion() {
        for (key, bad) in [
            ("stand", json!(176)),
            ("stand", json!(1001)),
            ("stand", json!(17.5)),
            ("stand", json!("175")),
            ("stance", json!("orbit")),
            ("wait", json!(1)),
            ("bearing", json!(360)),
            ("bearing", json!(-1)),
            ("facets", json!(["top", "front"])),
            ("facets", json!(["front", "front"])),
            ("facets", json!(["bottom"])),
            ("classes", json!(["red"])),
        ] {
            let mut values = widgets();
            values[2]["Default"][key] = bad;
            assert!(
                validate(&values, CoreClass::I).is_err(),
                "{key}: {}",
                values[2]
            );
        }
    }

    #[test]
    fn m6_schema_requires_every_value_and_refuses_unknown_keys() {
        let mut values = widgets();
        values[2]["Default"]
            .as_object_mut()
            .expect("profile")
            .remove("stand");
        assert_eq!(
            validate(&values, CoreClass::I).expect_err("missing").field,
            "profiles.Default.stand"
        );
        let mut values = widgets();
        values[2]["Default"]["secret"] = json!(1);
        assert_eq!(
            validate(&values, CoreClass::I).expect_err("unknown").field,
            "profiles.Default.secret"
        );
    }

    #[test]
    fn m6_schema_refuses_unknown_shapes_and_stations() {
        for pointer in [
            "/sections/0",
            "/sections/0/fields/0",
            "/sections/0/fields/0/widget",
        ] {
            let mut values = widgets();
            values[0].pointer_mut(pointer).expect("shape")["unknown"] = json!(0);
            assert!(validate(&values, CoreClass::I).is_err(), "{pointer}");
        }
        let mut values = widgets();
        values[0]["sections"][0]["station"] = json!("admin");
        assert!(validate(&values, CoreClass::I).is_err());
        values[0]["sections"][0]["station"] = json!("fire");
        validate(&values, CoreClass::I)
            .expect("no chassis query; absent fire station is inert later");
    }

    #[test]
    fn m6_schema_refuses_invalid_defaults_steps_and_identifiers() {
        for (key, bad) in [
            ("default", json!(177)),
            ("step", json!(0)),
            ("step", json!(-5)),
            ("scale", json!(3)),
            ("max", json!(1001)),
            ("min", json!(-9_000_000_000_i64)),
        ] {
            let mut values = widgets();
            values[0]["sections"][0]["fields"][0]["widget"][key] = bad;
            assert!(validate(&values, CoreClass::I).is_err(), "{key}");
        }
        let mut values = widgets();
        values[0]["sections"][0]["fields"][1]["id"] = json!("stand");
        assert!(
            validate(&values, CoreClass::I).is_err(),
            "duplicate field across schema"
        );
        values[0]["sections"][0]["fields"][1]["id"] = json!("bad key");
        assert!(
            validate(&values, CoreClass::I).is_err(),
            "unsafe identifier"
        );
    }

    #[test]
    fn m6_schema_profile_and_rule_caps_follow_all_four_classes() {
        for class in CoreClass::ALL {
            let caps = ClassTable::STOCK.row(class);
            let mut values = empty();
            values[2] = json!(
                (0..caps.profiles)
                    .map(|n| (format!("P{n}"), json!({})))
                    .collect::<BTreeMap<_, _>>()
            );
            values[3] = json!({"P0":vec![fire();caps.slots]});
            validate(&values, class).expect("at exact profile and rule caps");
            values[3]["P0"]
                .as_array_mut()
                .expect("rules")
                .push(json!(fire()));
            assert!(validate(&values, class).is_err(), "class {class:?} rules");
            values[3] = json!({});
            values[2][format!("P{}", caps.profiles)] = json!({});
            assert!(
                validate(&values, class).is_err(),
                "class {class:?} profiles"
            );
        }
        let mut values = empty();
        values[2] = json!({});
        assert!(
            validate(&values, CoreClass::III).is_err(),
            "one active profile is required"
        );
    }

    #[test]
    fn m6_schema_profile_rules_cannot_name_a_missing_or_ambiguous_profile() {
        let mut values = empty();
        values[3] = json!({"Missing":[]});
        assert!(validate(&values, CoreClass::I).is_err());
        values[3] = json!({});
        values[2]["default"] = json!({});
        assert!(
            validate(&values, CoreClass::I).is_err(),
            "case ambiguous names"
        );
    }

    #[test]
    fn m6_schema_native_and_think_terms_share_one_four_term_budget() {
        let mut rule = fire();
        if let Rule::Fire { predicates, .. } = &mut rule.rule {
            predicates.push(RulePredicate::Lock { present: true });
        }
        rule.think = vec![
            ThinkPredicate::HitWithin {
                milliseconds: 5_000,
            },
            ThinkPredicate::Hostiles { at_least: 2 },
            ThinkPredicate::Order {
                is: OrderKind::Advance,
            },
        ];
        validate(&with_rules(vec![rule.clone()]), CoreClass::I).expect("four total terms");
        if let Rule::Fire { predicates, .. } = &mut rule.rule {
            predicates.push(RulePredicate::Heat { above_percent: 80 });
        }
        assert!(
            validate(&with_rules(vec![rule]), CoreClass::I).is_err(),
            "five terms"
        );
    }

    #[test]
    fn m6_schema_refuses_internal_flags_live_contact_ids_and_compiled_hidden_terms() {
        for mutation in 0..4 {
            let mut rule = fire();
            if let Rule::Fire {
                condition,
                implicit,
                independent,
                ..
            } = &mut rule.rule
            {
                match mutation {
                    0 => *implicit = true,
                    1 => *independent = true,
                    2 => {
                        condition.target = SlotTarget::Contact {
                            id: super::super::state::ContactId(4),
                        }
                    }
                    _ => condition.min_rounds_left = 2,
                }
            }
            assert!(
                validate(&with_rules(vec![rule]), CoreClass::I).is_err(),
                "case {mutation}"
            );
        }
        let rule = AuthoredRule {
            rule: Rule::lock_rule(LockSelector::Nearest, None, vec![]),
            think: vec![],
        };
        validate(&with_rules(vec![rule]), CoreClass::I).expect("shared typed Sensors action");
    }

    #[test]
    fn m6_schema_nested_unknown_fields_and_unbounded_think_terms_are_refused() {
        let mut values = with_rules(vec![fire()]);
        values[3]["Default"][0]["rule"]["condition"]["target"]["secret"] = json!(1);
        assert!(
            validate(&values, CoreClass::I).is_err(),
            "nested reused type is strict too"
        );
        for think in [
            ThinkPredicate::HitWithin { milliseconds: 0 },
            ThinkPredicate::HitWithin {
                milliseconds: 300_001,
            },
            ThinkPredicate::Hostiles { at_least: 25 },
        ] {
            let mut rule = fire();
            rule.think.push(think);
            assert!(validate(&with_rules(vec![rule]), CoreClass::I).is_err());
        }
    }

    #[test]
    fn m6_schema_floor_is_refused_as_authored_and_has_no_hidden_fifth_term() {
        let rule = fire();
        let decoded = validate(&with_rules(vec![rule.clone()]), CoreClass::I).expect("shape");
        decoded.validate_floor(100).expect("at floor");
        assert!(decoded.validate_floor(110).is_err(), "never clamp");
        let mut hidden = rule;
        if let Rule::Fire { condition, .. } = &mut hidden.rule {
            condition.min_permille = 600;
        }
        let decoded =
            validate(&with_rules(vec![hidden]), CoreClass::I).expect("shape needs captured floor");
        assert!(
            decoded.validate_floor(100).is_err(),
            "explicit odds belong in the predicates"
        );
    }

    #[test]
    fn m6_schema_metadata_limits_apply_before_typed_decode() {
        let mut values = empty();
        values[0] = json!({"sections":[],"oversized":"x".repeat(8193)});
        assert!(validate(&values, CoreClass::I).is_err());
        let mut nested = json!(0);
        for _ in 0..18 {
            nested = json!([nested]);
        }
        values[0] = nested;
        assert!(validate(&values, CoreClass::I).is_err());
        values[0] = json!({"sections":[],"nodes":vec![0;4096]});
        assert!(validate(&values, CoreClass::I).is_err());
    }
    #[test]
    fn m6_rule_words_keep_native_and_sampled_terms_in_one_complete_sentence() {
        let rule = AuthoredRule {
            rule: crate::arena::conditions::Rule::hold_rule("cannon".into(), vec![]),
            think: vec![
                ThinkPredicate::HitWithin {
                    milliseconds: 1_000,
                },
                ThinkPredicate::Hostiles { at_least: 2 },
                ThinkPredicate::Order {
                    is: OrderKind::Hold,
                },
            ],
        };
        rule.validate().unwrap();
        let words = rule_words(&rule);
        assert!(words.starts_with(&crate::arena::rules::rule_words(&rule.rule, None)));
        assert!(words.contains("within 1000 ms"));
        assert!(words.contains("at least 2 classified living hostiles"));
        assert!(words.ends_with("your standing order is hold"));
        assert_eq!(words.matches(" when ").count(), 1);
        assert_eq!(words.matches(" and ").count(), 2);
    }
}
