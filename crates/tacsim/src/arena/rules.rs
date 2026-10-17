//! Bounded native station-rule grammar and its English readback.
//!
//! The caller supplies only its permitted frame projection. Parsing does not
//! inspect an arena or resolve a contact through world truth. Admission, slot
//! capacity, channel authority and fresh classification remain the arena's job.

use smol_str::SmolStr;

use super::conditions::{
    ASPECTS_ANY, Arming, Comparison, FireCondition, LockSelector, Rule, RuleBand, RulePredicate,
    SectionState, SlotTarget, TargetState,
};
use super::state::ContactId;

/// Maximum predicates, including a target-selector term, on one native rule.
pub const MAX_RULE_TERMS: usize = 4;
/// Maximum complete station-rule command, in UTF-8 bytes.
pub const MAX_RULE_BYTES: usize = 1_024;
/// Largest metre figure accepted by a native range term.
pub const MAX_RULE_RANGE_M: u32 = 10_000;
/// Maximum physical rule-slot identifier (the installed core may grant fewer).
pub const MAX_RULE_SLOT: u8 = 6;

/// The own-weapon facts needed to interpret a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleWeapon {
    /// Canonical component id, with the same initials/prefix aliases as FIRE.
    pub id: SmolStr,
    /// Full ammunition load: the denominator of a percentage rounds predicate.
    pub rounds_max: u32,
}

/// One contact name in the caller's permitted projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleContact {
    /// The designator the pilot can type. An alias may be a second row.
    pub name: SmolStr,
    /// Opaque contact id; never printed by the grammar's English formatter.
    pub id: ContactId,
    /// Whether the caller has earned identification now.
    pub classified: bool,
}

/// Frame facts used by the parser. The arena rechecks these at admission.
#[derive(Clone, Copy, Debug)]
pub struct RuleContext<'a> {
    /// Own weapons in component order.
    pub weapons: &'a [RuleWeapon],
    /// Captured Compact floor, in per mille.
    pub floor_permille: u16,
    /// Contact names visible to this principal, including any permitted aliases.
    pub contacts: &'a [RuleContact],
}

/// A parsed station command; read/remove slot lookup belongs to the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleCommand {
    /// Stage this arming through its own channel and authority epoch.
    Arm(Arming),
    /// Read every active rule, or one stable slot.
    Read {
        /// One-based slot; none requests the ordered list.
        slot: Option<u8>,
    },
    /// Remove a slot using that slot's actual channel, rechecked at landing.
    Remove {
        /// One-based slot.
        slot: u8,
    },
}

/// A safe, bounded native-grammar refusal.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RuleParseError {
    /// Outside the bounded native grammar.
    #[error("That rule does not fit this core's native grammar.")]
    Syntax,
    /// The condition requires an active authored mind's think.
    #[error("That condition needs a thinking mind; a pilot's mind is dormant.")]
    ThinkOnly,
    /// Compact odds floor is refused as typed, never silently clamped.
    #[error("The Compact floor is {floor_pct}%; a condition may not ask for less.")]
    Unlawful {
        /// Captured Compact floor, in whole percent.
        floor_pct: u16,
    },
    /// No carried weapon answers to the bounded symbol.
    #[error("No carried weapon answers to {0}.")]
    UnknownWeapon(SmolStr),
    /// More than one carried weapon answers to the bounded symbol.
    #[error("More than one carried weapon answers to {0}; use its full name.")]
    AmbiguousWeapon(SmolStr),
    /// No weapon is carried.
    #[error("This unit carries no weapon.")]
    NoWeapon,
    /// The designator is not in the caller's projection.
    #[error("That contact is not on your plot.")]
    UnknownTarget,
    /// Native rules cannot cite an unidentified contact.
    #[error("A rule may name only an identified contact.")]
    UnclassifiedTarget,
    /// Slot zero and slots above the largest core are not valid identifiers.
    #[error("A rule slot is a whole number from 1 to 6.")]
    InvalidSlot,
    /// One explicit slot cannot hold a multi-weapon expansion.
    #[error("An explicit slot holds one rule; name one weapon.")]
    MultipleInSlot,
}

/// Parse `FIRE weapon|ALL [HOLD] WHEN ...`, `LOCK selector WHEN ...`,
/// `FIRE [weapon|ALL] OFF`, or the internal `RULE[S]` forms.
///
/// The station dispatches `MIND RULES [slot]` as `RULES [slot]` and
/// `MIND RULE slot OFF|<rule>` as `RULE slot OFF|<rule>`. An unnumbered
/// arming is the single-rule shorthand; the runtime chooses/replaces its
/// implicit slot. An explicit slot replaces only that slot. `ALL` is one
/// atomic arming carrying one rule per weapon, never several queued mutations.
///
/// Immediate pilot FIRE is deliberately outside this parser. Range figures
/// are whole rounded metres and comparisons remain strict. Rounds percentages
/// use the full ammunition load, own sections their original maximum, and heat
/// the heat system's maximum. Unknown earned readings evaluate false.
///
/// # Errors
/// Returns a bounded refusal for malformed, unavailable or unlawful terms.
pub fn parse_command(text: &str, context: &RuleContext<'_>) -> Result<RuleCommand, RuleParseError> {
    let tokens = tokens(text)?;
    let words: Vec<&str> = tokens.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["rules"] => Ok(RuleCommand::Read { slot: None }),
        ["rules", slot] => Ok(RuleCommand::Read {
            slot: Some(slot_number(slot)?),
        }),
        ["rule", slot, "off"] => Ok(RuleCommand::Remove {
            slot: slot_number(slot)?,
        }),
        ["rule", slot, body @ ..] => parse_action(body, Some(slot_number(slot)?), context),
        body => parse_action(body, None, context),
    }
}

fn parse_action(
    words: &[&str],
    slot: Option<u8>,
    context: &RuleContext<'_>,
) -> Result<RuleCommand, RuleParseError> {
    match words {
        ["fire", "off"] | ["fire", "all", "off"] if slot.is_none() => {
            Ok(RuleCommand::Arm(Arming::Disarm { weapon: None }))
        }
        ["fire", named, "off"] if slot.is_none() => {
            let weapon = weapon(context, named)?;
            Ok(RuleCommand::Arm(Arming::Disarm {
                weapon: Some(weapon.id.clone()),
            }))
        }
        ["fire", named, "when", terms @ ..] => fire_rules(named, false, terms, slot, context),
        ["fire", named, "hold", "when", terms @ ..] => {
            fire_rules(named, true, terms, slot, context)
        }
        ["lock", named, "when", terms @ ..] => {
            let selector = match *named {
                "nearest" => LockSelector::Nearest,
                "best_odds" => LockSelector::BestOdds,
                "weakest" => LockSelector::Weakest,
                "flank" => LockSelector::Flank,
                "newest" => LockSelector::Newest,
                _ => return Err(RuleParseError::Syntax),
            };
            let default = context
                .weapons
                .iter()
                .find(|weapon| weapon.id == super::weapons::DEFAULT_WEAPON)
                .or_else(|| context.weapons.first());
            let (predicates, target) = predicates(terms, default, context)?;
            if target.is_some() {
                return Err(RuleParseError::Syntax);
            }
            check_floor(&predicates, context)?;
            let selected = predicates.iter().find_map(|predicate| match predicate {
                RulePredicate::Odds { weapon, .. } => weapon.clone(),
                _ => None,
            });
            Ok(RuleCommand::Arm(Arming::Rules {
                slot,
                rules: vec![Rule::lock_rule(selector, selected, predicates)],
            }))
        }
        _ => Err(RuleParseError::Syntax),
    }
}

fn fire_rules(
    named: &str,
    hold: bool,
    terms: &[&str],
    slot: Option<u8>,
    context: &RuleContext<'_>,
) -> Result<RuleCommand, RuleParseError> {
    let weapons = if named == "all" {
        if context.weapons.is_empty() {
            return Err(RuleParseError::NoWeapon);
        }
        context.weapons.iter().collect::<Vec<_>>()
    } else {
        vec![weapon(context, named)?]
    };
    if slot.is_some() && weapons.len() != 1 {
        return Err(RuleParseError::MultipleInSlot);
    }
    let mut rules = Vec::with_capacity(weapons.len());
    for weapon in weapons {
        let (predicates, target) = predicates(terms, Some(weapon), context)?;
        check_floor(&predicates, context)?;
        if hold {
            if target.is_some() {
                return Err(RuleParseError::Syntax);
            }
            rules.push(Rule::hold_rule(weapon.id.clone(), predicates));
        } else {
            let min_permille = predicates
                .iter()
                .filter_map(|predicate| match predicate {
                    RulePredicate::Odds {
                        weapon: named,
                        min_permille,
                    } if named.as_ref().is_none_or(|named| *named == weapon.id) => {
                        Some(*min_permille)
                    }
                    _ => None,
                })
                .max()
                .unwrap_or(context.floor_permille)
                .max(context.floor_permille);
            rules.push(Rule::fire_rule(
                FireCondition {
                    weapon: weapon.id.clone(),
                    target: target.unwrap_or(SlotTarget::Lock),
                    min_permille,
                    aspects: ASPECTS_ANY,
                    min_rounds_left: 0,
                    min_range_m: 0,
                    max_range_m: u32::MAX,
                },
                predicates,
            ));
        }
    }
    Ok(RuleCommand::Arm(Arming::Rules { slot, rules }))
}

fn predicates(
    words: &[&str],
    default: Option<&RuleWeapon>,
    context: &RuleContext<'_>,
) -> Result<(Vec<RulePredicate>, Option<SlotTarget>), RuleParseError> {
    if words.is_empty() || words.contains(&"or") {
        return Err(RuleParseError::Syntax);
    }
    let terms: Vec<&[&str]> = words.split(|word| *word == "and").collect();
    if terms.len() > MAX_RULE_TERMS || terms.iter().any(|term| term.is_empty()) {
        return Err(RuleParseError::Syntax);
    }
    let mut result = Vec::with_capacity(terms.len());
    let mut target = None;
    for term in terms {
        if let ["target", "=", name] = term
            && !matches!(*name, "identified" | "moving" | "stopped" | "hurt")
        {
            let selected = match *name {
                "lock" => SlotTarget::Lock,
                "best_hostile" => SlotTarget::BestHostile,
                name => {
                    let contact = context
                        .contacts
                        .iter()
                        .find(|contact| contact.name.eq_ignore_ascii_case(name))
                        .ok_or(RuleParseError::UnknownTarget)?;
                    if !contact.classified {
                        return Err(RuleParseError::UnclassifiedTarget);
                    }
                    SlotTarget::Contact { id: contact.id }
                }
            };
            if target.as_ref().is_some_and(|held| *held != selected) {
                return Err(RuleParseError::Syntax);
            }
            target = Some(selected);
            continue;
        }
        let parsed = predicate(term, default, context)?;
        if result
            .iter()
            .any(|held| same_term(held, &parsed) && *held != parsed)
        {
            return Err(RuleParseError::Syntax);
        }
        if !result.contains(&parsed) {
            result.push(parsed);
        }
    }
    Ok((result, target))
}

fn predicate(
    term: &[&str],
    default: Option<&RuleWeapon>,
    context: &RuleContext<'_>,
) -> Result<RulePredicate, RuleParseError> {
    match term {
        ["hit", ..] | ["hostiles", ..] | ["order", ..] => Err(RuleParseError::ThinkOnly),
        ["odds", ">=", value] => Ok(RulePredicate::Odds {
            weapon: None,
            min_permille: percent(value)? * 10,
        }),
        ["odds", named, ">=", value] => Ok(RulePredicate::Odds {
            weapon: Some(weapon(context, named)?.id.clone()),
            min_permille: percent(value)? * 10,
        }),
        ["aspect", "=", values @ ..] => {
            if values.is_empty() || values.len() % 2 == 0 {
                return Err(RuleParseError::Syntax);
            }
            let mut mask = 0;
            for (index, value) in values.iter().enumerate() {
                if index % 2 == 1 {
                    if *value != "," {
                        return Err(RuleParseError::Syntax);
                    }
                } else {
                    mask |= match *value {
                        "front" => 0b0001,
                        "side" => 0b0110,
                        "rear" => 0b1000,
                        _ => return Err(RuleParseError::Syntax),
                    };
                }
            }
            Ok(RulePredicate::Aspect { mask })
        }
        ["range", op @ ("<" | ">"), value] | ["range", op @ ("<" | ">"), value, "m"] => {
            Ok(RulePredicate::Range {
                comparison: comparison(op)?,
                metres: number(
                    value.strip_suffix('m').unwrap_or(value),
                    1,
                    MAX_RULE_RANGE_M,
                )?,
            })
        }
        ["band", "=", value] => {
            let _ = default.ok_or(RuleParseError::NoWeapon)?;
            Ok(RulePredicate::Band {
                band: match *value {
                    "close" => RuleBand::Close,
                    "effective" => RuleBand::Effective,
                    "far" => RuleBand::Far,
                    _ => return Err(RuleParseError::Syntax),
                },
            })
        }
        ["target", "=", value] => Ok(RulePredicate::Target {
            state: match *value {
                "identified" => TargetState::Identified,
                "moving" => TargetState::Moving,
                "stopped" => TargetState::Stopped,
                "hurt" => TargetState::Hurt,
                _ => return Err(RuleParseError::Syntax),
            },
        }),
        ["target", section, "=", value] => Ok(RulePredicate::TargetSection {
            section: symbol(section)?,
            state: match *value {
                "green" => SectionState::Green,
                "yellow" => SectionState::Yellow,
                "red" => SectionState::Red,
                "destroyed" => SectionState::Destroyed,
                _ => return Err(RuleParseError::Syntax),
            },
        }),
        ["rounds", op @ ("<" | ">"), value] => rounds(default, None, op, value, false),
        ["rounds", op @ ("<" | ">"), value, "%"] => rounds(default, None, op, value, true),
        ["rounds", named, op @ ("<" | ">"), value] => {
            let chosen = weapon(context, named)?;
            rounds(Some(chosen), Some(chosen.id.clone()), op, value, false)
        }
        ["rounds", named, op @ ("<" | ">"), value, "%"] => {
            let chosen = weapon(context, named)?;
            rounds(Some(chosen), Some(chosen.id.clone()), op, value, true)
        }
        ["reserve"] => rounds(default, None, ">", "0", false),
        ["own", section, "<", value] | ["own", section, "<", value, "%"] => {
            if term.len() == 4 && !value.ends_with('%') {
                return Err(RuleParseError::Syntax);
            }
            Ok(RulePredicate::OwnSection {
                section: symbol(section)?,
                below_percent: small_percent(value)?,
            })
        }
        ["heat", ">", value] | ["heat", ">", value, "%"] => {
            if term.len() == 3 && !value.ends_with('%') {
                return Err(RuleParseError::Syntax);
            }
            Ok(RulePredicate::Heat {
                above_percent: small_percent(value)?,
            })
        }
        ["lock"] => Ok(RulePredicate::Lock { present: true }),
        ["no", "lock"] | ["nolock"] => Ok(RulePredicate::Lock { present: false }),
        _ => Err(RuleParseError::Syntax),
    }
}

fn rounds(
    chosen: Option<&RuleWeapon>,
    weapon: Option<SmolStr>,
    op: &str,
    value: &str,
    separate_percent: bool,
) -> Result<RulePredicate, RuleParseError> {
    let chosen = chosen.ok_or(RuleParseError::NoWeapon)?;
    let percent = separate_percent || value.ends_with('%');
    let amount = number(
        value.strip_suffix('%').unwrap_or(value),
        0,
        if percent { 100 } else { chosen.rounds_max },
    )?;
    Ok(RulePredicate::Rounds {
        weapon,
        comparison: comparison(op)?,
        amount,
        percent,
    })
}

fn same_term(left: &RulePredicate, right: &RulePredicate) -> bool {
    match (left, right) {
        (RulePredicate::Target { state: a }, RulePredicate::Target { state: b }) => {
            // Identification, motion and damage are independent earned facts.
            // The two motion states alone contradict each other.
            a == b
                || matches!(
                    (a, b),
                    (TargetState::Moving, TargetState::Stopped)
                        | (TargetState::Stopped, TargetState::Moving)
                )
        }
        (RulePredicate::Odds { weapon: a, .. }, RulePredicate::Odds { weapon: b, .. }) => a == b,
        (
            RulePredicate::Range { comparison: a, .. },
            RulePredicate::Range { comparison: b, .. },
        ) => a == b,
        (
            RulePredicate::Rounds {
                weapon: a,
                comparison: x,
                ..
            },
            RulePredicate::Rounds {
                weapon: b,
                comparison: y,
                ..
            },
        ) => a == b && x == y,
        (
            RulePredicate::TargetSection { section: a, .. },
            RulePredicate::TargetSection { section: b, .. },
        )
        | (
            RulePredicate::OwnSection { section: a, .. },
            RulePredicate::OwnSection { section: b, .. },
        ) => a == b,
        _ => std::mem::discriminant(left) == std::mem::discriminant(right),
    }
}

fn check_floor(
    predicates: &[RulePredicate],
    context: &RuleContext<'_>,
) -> Result<(), RuleParseError> {
    if predicates.iter().any(|predicate| {
        matches!(predicate,
            RulePredicate::Odds { min_permille, .. } if *min_permille < context.floor_permille
        )
    }) {
        return Err(RuleParseError::Unlawful {
            floor_pct: context.floor_permille / 10,
        });
    }
    Ok(())
}

fn weapon<'a>(context: &'a RuleContext<'_>, named: &str) -> Result<&'a RuleWeapon, RuleParseError> {
    let named = symbol(named)?;
    for kind in 0..3 {
        let mut found = context.weapons.iter().filter(|row| match kind {
            0 => row.id.eq_ignore_ascii_case(&named),
            1 => {
                let parts: Vec<_> = row.id.split('_').filter(|part| !part.is_empty()).collect();
                parts.len() > 1
                    && parts
                        .iter()
                        .filter_map(|part| part.chars().next())
                        .collect::<String>()
                        .eq_ignore_ascii_case(&named)
            }
            _ => row.id.to_ascii_lowercase().starts_with(named.as_str()),
        });
        if let Some(one) = found.next() {
            if found.next().is_some() {
                return Err(RuleParseError::AmbiguousWeapon(named));
            }
            return Ok(one);
        }
    }
    Err(RuleParseError::UnknownWeapon(named))
}

fn comparison(value: &str) -> Result<Comparison, RuleParseError> {
    match value {
        "<" => Ok(Comparison::Less),
        ">" => Ok(Comparison::Greater),
        _ => Err(RuleParseError::Syntax),
    }
}

fn slot_number(value: &str) -> Result<u8, RuleParseError> {
    number(value, 1, u32::from(MAX_RULE_SLOT))
        .and_then(|value| u8::try_from(value).map_err(|_| RuleParseError::InvalidSlot))
        .map_err(|_| RuleParseError::InvalidSlot)
}

fn number(value: &str, minimum: u32, maximum: u32) -> Result<u32, RuleParseError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RuleParseError::Syntax);
    }
    value
        .parse()
        .ok()
        .filter(|value| (minimum..=maximum).contains(value))
        .ok_or(RuleParseError::Syntax)
}

fn percent(value: &str) -> Result<u16, RuleParseError> {
    u16::try_from(number(value, 1, 100)?).map_err(|_| RuleParseError::Syntax)
}

fn small_percent(value: &str) -> Result<u8, RuleParseError> {
    u8::try_from(number(value.strip_suffix('%').unwrap_or(value), 0, 100)?)
        .map_err(|_| RuleParseError::Syntax)
}

fn symbol(value: &str) -> Result<SmolStr, RuleParseError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(RuleParseError::Syntax);
    }
    Ok(SmolStr::new(value))
}

fn tokens(text: &str) -> Result<Vec<String>, RuleParseError> {
    if text.len() > MAX_RULE_BYTES || !text.is_ascii() {
        return Err(RuleParseError::Syntax);
    }
    let mut result = Vec::new();
    let mut word = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let operator = match c {
            '>' if chars.peek() == Some(&'=') => {
                chars.next();
                Some(">=")
            }
            '>' => Some(">"),
            '<' => Some("<"),
            '=' => Some("="),
            ',' => Some(","),
            _ => None,
        };
        if c.is_ascii_whitespace() || operator.is_some() {
            if !word.is_empty() {
                result.push(std::mem::take(&mut word));
            }
            if let Some(operator) = operator {
                result.push(operator.to_owned());
            }
        } else {
            word.push(c.to_ascii_lowercase());
        }
    }
    if !word.is_empty() {
        result.push(word);
    }
    if result.len() > 128 {
        return Err(RuleParseError::Syntax);
    }
    Ok(result)
}

/// Read back one native rule without revealing opaque contact identity.
///
/// The optional target name must come from the caller's current earned plot.
/// A lost/unclassified name is rendered generically, never as its numeric id.
#[must_use]
pub fn rule_words(rule: &Rule, target_name: Option<&str>) -> String {
    let action = match rule {
        Rule::Fire { condition, .. } => {
            let target = match condition.target {
                SlotTarget::Lock => "your identified lock",
                SlotTarget::BestHostile => "your identified hostile lock",
                SlotTarget::Contact { .. } => target_name.unwrap_or("the identified contact"),
            };
            format!("{} fires at {target}", condition.weapon)
        }
        Rule::HoldFire { weapon, .. } => format!("{weapon} holds fire"),
        Rule::Lock { selector, .. } => match selector {
            LockSelector::Nearest => "lock the nearest identified hostile",
            LockSelector::BestOdds => "lock the identified hostile with the best hit chance",
            LockSelector::Weakest => "lock the identified hostile with the most observed damage",
            LockSelector::Flank => "lock the identified hostile showing the most exposed face",
            LockSelector::Newest => "lock the identified hostile with the freshest plot",
        }
        .to_owned(),
    };
    let mut clauses = rule
        .predicates()
        .iter()
        .filter(|predicate| {
            !matches!((rule, predicate),
                (Rule::Fire { condition, .. }, RulePredicate::Odds { weapon, min_permille })
                    if weapon.as_ref().is_none_or(|weapon| *weapon == condition.weapon)
                        && *min_permille <= condition.min_permille
            )
        })
        .map(predicate_words)
        .collect::<Vec<_>>();
    if let Rule::Fire { condition, .. } = rule {
        clauses.insert(
            0,
            format!(
                "its chance to hit is at least {}%",
                condition.min_permille / 10
            ),
        );
        // Legacy shorthand and Standard share this table. Their constraints
        // live in FireCondition, so a complete read must include those too.
        if condition.aspects & ASPECTS_ANY != ASPECTS_ANY {
            clauses.push(predicate_words(&RulePredicate::Aspect {
                mask: condition.aspects,
            }));
        }
        if condition.min_range_m > 0 {
            clauses.push(format!(
                "its rounded range is at least {} m",
                condition.min_range_m
            ));
        }
        if condition.max_range_m < u32::MAX {
            clauses.push(format!(
                "its rounded range is at most {} m",
                condition.max_range_m
            ));
        }
        if condition.min_rounds_left > 0 {
            clauses.push(format!(
                "{} keeps {} round(s)",
                condition.weapon, condition.min_rounds_left
            ));
        }
    }
    if clauses.is_empty() {
        action
    } else {
        format!("{action} when {}", clauses.join(" and "))
    }
}

fn comparison_words(comparison: &Comparison) -> &'static str {
    match comparison {
        Comparison::Less => "less than",
        Comparison::Greater => "more than",
        Comparison::AtLeast => "at least",
        Comparison::Equal => "equal to",
    }
}

fn predicate_words(predicate: &RulePredicate) -> String {
    match predicate {
        RulePredicate::Odds {
            weapon,
            min_permille,
        } => format!(
            "{}'s chance to hit is at least {}%",
            weapon.as_deref().unwrap_or("the selected weapon"),
            min_permille / 10
        ),
        RulePredicate::Aspect { mask } => {
            let faces = [(0b0001, "front"), (0b0110, "side"), (0b1000, "rear")]
                .into_iter()
                .filter_map(|(bits, name)| (mask & bits != 0).then_some(name))
                .collect::<Vec<_>>();
            format!("the target shows its {}", faces.join(" or "))
        }
        RulePredicate::Range { comparison, metres } => format!(
            "its rounded range is {} {metres} m",
            comparison_words(comparison)
        ),
        RulePredicate::Band { band } => format!(
            "its range band is {}",
            match band {
                RuleBand::Close => "close",
                RuleBand::Effective => "effective",
                RuleBand::Far => "far",
            }
        ),
        RulePredicate::Target { state } => format!(
            "the target is {}",
            match state {
                TargetState::Identified => "identified",
                TargetState::Moving => "moving",
                TargetState::Stopped => "stopped",
                TargetState::Hurt => "hurt",
            }
        ),
        RulePredicate::TargetSection { section, state } => format!(
            "the earned {section} reading is {}",
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
        } => format!(
            "{} has {} {amount}{}",
            weapon.as_deref().unwrap_or("the selected weapon"),
            comparison_words(comparison),
            if *percent {
                "% of its full load"
            } else {
                " rounds"
            }
        ),
        RulePredicate::OwnSection {
            section,
            below_percent,
        } => format!("your {section} internals are below {below_percent}% of their maximum"),
        RulePredicate::Heat { above_percent } => {
            format!("your heat is above {above_percent}% of its maximum")
        }
        RulePredicate::Lock { present: true } => "you have an identified lock".to_owned(),
        RulePredicate::Lock { present: false } => "you have no identified lock".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weapons() -> Vec<RuleWeapon> {
        vec![
            RuleWeapon {
                id: "cannon".into(),
                rounds_max: 41,
            },
            RuleWeapon {
                id: "machine_gun".into(),
                rounds_max: 300,
            },
        ]
    }

    fn parse(text: &str) -> Result<RuleCommand, RuleParseError> {
        parse_command(
            text,
            &RuleContext {
                weapons: &weapons(),
                floor_permille: 100,
                contacts: &[],
            },
        )
    }

    fn one(text: &str) -> Rule {
        let RuleCommand::Arm(Arming::Rules { mut rules, .. }) = parse(text).unwrap() else {
            panic!("expected rules");
        };
        assert_eq!(rules.len(), 1);
        rules.remove(0)
    }

    #[test]
    fn strict_units_and_percentages_survive_parsing_without_rounding() {
        let rule = one("FIRE CAN WHEN range<600m AND rounds>25% AND own turret<40 % AND heat>80%");
        assert_eq!(
            rule.predicates(),
            &[
                RulePredicate::Range {
                    comparison: Comparison::Less,
                    metres: 600
                },
                RulePredicate::Rounds {
                    weapon: None,
                    comparison: Comparison::Greater,
                    amount: 25,
                    percent: true
                },
                RulePredicate::OwnSection {
                    section: "turret".into(),
                    below_percent: 40
                },
                RulePredicate::Heat { above_percent: 80 },
            ]
        );
        let words = rule_words(&rule, None);
        assert!(words.contains("less than 600 m"));
        assert!(words.contains("25% of its full load"));
        assert!(!words.contains("599"));
    }

    #[test]
    fn all_is_one_atomic_command_and_a_numbered_slot_is_one_rule() {
        let RuleCommand::Arm(Arming::Rules { slot, rules }) =
            parse("FIRE ALL HOLD WHEN target=moving").unwrap()
        else {
            panic!("expected rules");
        };
        assert_eq!(slot, None);
        assert_eq!(rules.len(), 2);
        assert!(matches!(&rules[0], Rule::HoldFire { weapon, .. } if weapon == "cannon"));
        assert_eq!(
            parse("RULE 2 FIRE ALL WHEN lock"),
            Err(RuleParseError::MultipleInSlot)
        );
        assert!(matches!(
            parse("RULE 2 FIRE mg WHEN lock"),
            Ok(RuleCommand::Arm(Arming::Rules { slot: Some(2), .. }))
        ));
    }

    #[test]
    fn slots_are_explicit_and_old_off_is_still_disarming() {
        assert_eq!(parse("RULES"), Ok(RuleCommand::Read { slot: None }));
        assert_eq!(parse("RULES 6"), Ok(RuleCommand::Read { slot: Some(6) }));
        assert_eq!(parse("RULE 2 OFF"), Ok(RuleCommand::Remove { slot: 2 }));
        assert_eq!(
            parse("FIRE mg OFF"),
            Ok(RuleCommand::Arm(Arming::Disarm {
                weapon: Some("machine_gun".into())
            }))
        );
        assert_eq!(
            parse("FIRE ALL OFF"),
            Ok(RuleCommand::Arm(Arming::Disarm { weapon: None }))
        );
        for text in ["RULES 0", "RULE 7 OFF", "RULE -1 OFF"] {
            assert_eq!(parse(text), Err(RuleParseError::InvalidSlot));
        }
        assert_eq!(parse("FIRE cannon"), Err(RuleParseError::Syntax));
    }

    #[test]
    fn five_selectors_and_earned_predicates_are_native() {
        for action in ["FIRE cannon", "FIRE cannon HOLD", "LOCK nearest"] {
            let alias = one(&format!("{action} WHEN nolock"));
            assert_eq!(alias, one(&format!("{action} WHEN no lock")));
            assert!(rule_words(&alias, None).contains("you have no identified lock"));
        }
        for (word, expected) in [
            ("nearest", LockSelector::Nearest),
            ("best_odds", LockSelector::BestOdds),
            ("weakest", LockSelector::Weakest),
            ("flank", LockSelector::Flank),
            ("newest", LockSelector::Newest),
        ] {
            assert!(
                matches!(one(&format!("LOCK {word} WHEN no lock")), Rule::Lock { selector, .. } if selector == expected)
            );
        }
        assert_eq!(
            one("FIRE cannon WHEN target=identified AND target=moving AND target=hurt AND lock")
                .predicates()
                .len(),
            4
        );
        assert_eq!(
            one("FIRE cannon WHEN target=identified AND target=stopped")
                .predicates()
                .len(),
            2
        );
        assert_eq!(
            one("FIRE cannon WHEN target=moving AND target=moving")
                .predicates()
                .len(),
            1
        );
        assert_eq!(
            parse("FIRE cannon WHEN target=moving AND target=stopped"),
            Err(RuleParseError::Syntax)
        );
        for text in [
            "target=identified",
            "target=moving",
            "target=stopped",
            "target=hurt",
            "target turret=green",
            "target turret=yellow",
            "target turret=red",
            "target turret=destroyed",
            "band=close",
            "band=effective",
            "band=far",
            "rounds mg < 20",
            "reserve",
            "aspect=side,rear",
        ] {
            assert_eq!(
                one(&format!("FIRE cannon WHEN {text}")).predicates().len(),
                1,
                "{text}"
            );
        }
    }

    #[test]
    fn malformed_and_think_only_terms_fail_before_the_floor_check() {
        for term in ["hit within 2 s", "hostiles>=2", "order=hold"] {
            assert_eq!(
                parse(&format!("FIRE cannon WHEN {term}")),
                Err(RuleParseError::ThinkOnly)
            );
        }
        assert_eq!(
            parse("FIRE cannon WHEN odds>=1 AND nope"),
            Err(RuleParseError::Syntax)
        );
        assert_eq!(
            parse("FIRE cannon WHEN odds>=1"),
            Err(RuleParseError::Unlawful { floor_pct: 10 })
        );
        for text in [
            "FIRE cannon WHEN",
            "FIRE cannon WHEN lock OR no lock",
            "FIRE cannon WHEN lock AND",
            "FIRE cannon WHEN AND lock",
            "FIRE cannon WHEN lock AND lock AND lock AND lock AND lock",
            "FIRE cannon WHEN rounds<42",
            "FIRE cannon WHEN heat>101%",
            "FIRE cannon WHEN own hull<10",
            "FIRE cannon WHEN range<10001",
            "FIRE cannon WHEN range<0",
            "FIRE cannon WHEN odds>=101",
            "FIRE cannon WHEN range<1.5",
            "FIRE cannon WHEN heat>20% AND heat>30%",
        ] {
            assert_eq!(parse(text), Err(RuleParseError::Syntax), "{text}");
        }
    }

    #[test]
    fn contact_citation_requires_an_earned_name_and_never_echoes_an_id() {
        let contacts = [
            RuleContact {
                name: "BA".into(),
                id: ContactId(9_887_766),
                classified: true,
            },
            RuleContact {
                name: "X2".into(),
                id: ContactId(44),
                classified: false,
            },
        ];
        let weapons = weapons();
        let context = RuleContext {
            weapons: &weapons,
            floor_permille: 100,
            contacts: &contacts,
        };
        let RuleCommand::Arm(Arming::Rules { rules, .. }) =
            parse_command("FIRE cannon WHEN target=BA", &context).unwrap()
        else {
            panic!("expected rules");
        };
        assert!(
            matches!(&rules[0], Rule::Fire { condition, .. } if condition.target == SlotTarget::Contact { id: ContactId(9_887_766) })
        );
        assert!(!rule_words(&rules[0], None).contains("9887766"));
        assert!(rule_words(&rules[0], Some("BA")).contains("BA"));
        assert_eq!(
            parse_command("FIRE cannon WHEN target=X2", &context),
            Err(RuleParseError::UnclassifiedTarget)
        );
        assert_eq!(
            parse_command("FIRE cannon WHEN target=ZZ", &context),
            Err(RuleParseError::UnknownTarget)
        );
    }

    #[test]
    fn names_and_command_work_are_bounded() {
        assert_eq!(
            parse(&"x".repeat(MAX_RULE_BYTES + 1)),
            Err(RuleParseError::Syntax)
        );
        assert_eq!(
            parse("FIRE cannon WHEN target=é"),
            Err(RuleParseError::Syntax)
        );
        let weapons = [
            RuleWeapon {
                id: "machine_gun".into(),
                rounds_max: 10,
            },
            RuleWeapon {
                id: "missile_guidance".into(),
                rounds_max: 10,
            },
        ];
        let context = RuleContext {
            weapons: &weapons,
            floor_permille: 100,
            contacts: &[],
        };
        assert_eq!(
            parse_command("FIRE mg WHEN lock", &context),
            Err(RuleParseError::AmbiguousWeapon("mg".into()))
        );
        assert!(parse_command("FIRE machine_gun WHEN lock", &context).is_ok());
    }
}
