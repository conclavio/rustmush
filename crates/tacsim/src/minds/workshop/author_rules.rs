//! Author-only extension of the native grammar. No returned row is installed
//! here; a think term gates the complete action, including all native terms.

use crate::arena::conditions::{Arming, Rule};
use crate::arena::rules::{
    MAX_RULE_BYTES, MAX_RULE_TERMS, RuleCommand, RuleContext, RuleWeapon, parse_command,
};
use crate::arena::schema::{AuthoredRule, OrderKind, ThinkPredicate};
use mush_roll::RollError;

use super::invalid;

/// The native action and native terms keep their canonical English renderer;
/// think terms append to that same conjunction without inventing another action.
pub(crate) fn words(authored: &AuthoredRule) -> String {
    crate::arena::schema::rule_words(authored)
}

/// Parse one explicit ordered profile row. ALL remains symbolic until checkout
/// supplies a hull's own hardware; authored rows cannot capture contact ids.
pub(crate) fn parse(text: &str, floor_permille: u16) -> Result<AuthoredRule, RollError> {
    if text.len() > MAX_RULE_BYTES || !text.is_ascii() {
        return Err(invalid("rule", "maximum 1024 ASCII bytes"));
    }
    let words = text
        .split_ascii_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    let when = words
        .iter()
        .position(|word| word == "when")
        .ok_or_else(|| invalid("rule", "expected WHEN"))?;
    let mut action_words = words.get(..when).unwrap_or_default().to_vec();
    let all = action_words.first().is_some_and(|word| word == "fire")
        && action_words.get(1).is_some_and(|word| word == "all");
    // Authored operands are canonical symbols, not aliases resolved against an
    // arbitrary test hull. Native syntax validation receives only names written
    // in this row, with no magazine-specific ceiling. Membership is sealed later.
    // Percentages remain their raw semantic amount/percent pair throughout.
    const DEFAULT: &str = "__authored_default";
    if words.iter().any(|word| word.contains(DEFAULT)) {
        return Err(invalid("rule", "reserved parser operand"));
    }
    let mut own_weapons = vec![RuleWeapon {
        id: DEFAULT.into(),
        rounds_max: u32::MAX,
    }];
    if all {
        if let Some(operand) = action_words.get_mut(1) {
            *operand = DEFAULT.into();
        }
    } else if action_words.first().is_some_and(|word| word == "fire")
        && let Some(named) = action_words.get(1)
    {
        own_weapons.push(RuleWeapon {
            id: named.as_str().into(),
            rounds_max: u32::MAX,
        });
    }
    let action = action_words.join(" ");
    let conditions = words.get(when + 1..).unwrap_or_default();
    let terms = conditions.split(|word| word == "and").collect::<Vec<_>>();
    if terms.is_empty() || terms.len() > MAX_RULE_TERMS || terms.iter().any(|term| term.is_empty())
    {
        return Err(invalid("rule", "expected one through four AND terms"));
    }
    let mut native = Vec::new();
    let mut think = Vec::new();
    for term in terms {
        let text = term.join(" ");
        let normalized = text
            .replace(">=", " GE ")
            .replace('=', " = ")
            .replace(" GE ", " >= ");
        let words = normalized.split_ascii_whitespace().collect::<Vec<_>>();
        let predicate = match words.as_slice() {
            ["hit", "within", duration] => {
                let milliseconds = if let Some(value) = duration.strip_suffix("ms") {
                    unsigned(value, 300_000)?
                } else if let Some(value) = duration.strip_suffix('s') {
                    unsigned(value, 300)?
                        .checked_mul(1000)
                        .ok_or_else(|| invalid("rule", "duration overflow"))?
                } else {
                    return Err(invalid("rule", "hit duration needs ms or s"));
                };
                ThinkPredicate::HitWithin { milliseconds }
            }
            ["hostiles", ">=", amount] => ThinkPredicate::Hostiles {
                at_least: u8::try_from(unsigned(amount, 24)?)
                    .map_err(|_| invalid("rule", "hostile count"))?,
            },
            ["order", "=", kind] => ThinkPredicate::Order {
                is: match *kind {
                    "none" => OrderKind::None,
                    "hold" => OrderKind::Hold,
                    "advance" => OrderKind::Advance,
                    "engage" => OrderKind::Engage,
                    "withdraw" => OrderKind::Withdraw,
                    "screen" => OrderKind::Screen,
                    "support" => OrderKind::Support,
                    _ => return Err(invalid("rule", "unknown order kind")),
                },
            },
            ["hit" | "hostiles" | "order", ..] => {
                return Err(invalid("rule", "malformed think condition"));
            }
            _ => {
                // At most one explicit operand per bounded native term. The
                // native parser still validates complete syntax, symbol shape,
                // duplicates, numbers and floor; this table grants no hardware.
                let lhs = text.split(['<', '>', '=']).next().unwrap_or_default();
                let lhs = lhs.split_ascii_whitespace().collect::<Vec<_>>();
                if let ["odds" | "rounds", named] = lhs.as_slice() {
                    if *named == "all" {
                        return Err(invalid("rule", "ALL is only an action operand"));
                    }
                    if !own_weapons.iter().any(|weapon| weapon.id == *named) {
                        own_weapons.push(RuleWeapon {
                            id: (*named).into(),
                            rounds_max: u32::MAX,
                        });
                    }
                }
                native.push(text);
                continue;
            }
        };
        think.push(predicate);
    }
    // Native grammar requires a term. A temporary tautology is not exposed or
    // installed: remove it from the returned typed action before validation.
    let temporary_term = native.is_empty();
    let native_terms = if temporary_term {
        "lock".to_owned()
    } else {
        native.join(" AND ")
    };
    let own_only = RuleContext {
        weapons: &own_weapons,
        floor_permille,
        contacts: &[],
    };
    let parsed = parse_command(&format!("RULE 1 {action} WHEN {native_terms}"), &own_only)
        .map_err(|error| invalid("rule", error.to_string()))?;
    let RuleCommand::Arm(Arming::Rules { mut rules, .. }) = parsed else {
        return Err(invalid("rule", "expected one FIRE, HOLD or LOCK action"));
    };
    if rules.len() != 1 {
        return Err(invalid("rule", "name exactly one weapon"));
    }
    let mut rule = rules
        .pop()
        .ok_or_else(|| invalid("rule", "missing action"))?;
    if temporary_term {
        match &mut rule {
            Rule::Fire { predicates, .. }
            | Rule::HoldFire { predicates, .. }
            | Rule::Lock { predicates, .. } => predicates.clear(),
        }
    }
    if all {
        match &mut rule {
            Rule::Fire { condition, .. } => condition.weapon = "all".into(),
            Rule::HoldFire { weapon, .. } => *weapon = "all".into(),
            Rule::Lock { .. } => return Err(invalid("rule", "ALL is only a fire-station action")),
        }
    }
    let authored = AuthoredRule { rule, think };
    authored
        .validate()
        .map_err(|error| invalid("rule", error.to_string()))?;
    Ok(authored)
}

fn unsigned(word: &str, max: u32) -> Result<u32, RollError> {
    word.parse::<u32>()
        .ok()
        .filter(|value| (1..=max).contains(value) && word.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| invalid("rule", "condition number is out of range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_stays_symbolic_until_the_hull_is_sealed() {
        let authored = parse(
            "FIRE ALL WHEN rounds > 25% AND odds >= 60 AND odds laser >= 70",
            100,
        )
        .unwrap();
        assert_eq!(
            authored.rule.weapon().map(|weapon| weapon.as_str()),
            Some("all")
        );
        assert_eq!(authored.rule.fire().unwrap().min_permille, 600);
        assert!(authored.rule.predicates().iter().any(|predicate| matches!(
            predicate,
            crate::arena::conditions::RulePredicate::Rounds {
                weapon: None,
                amount: 25,
                percent: true,
                ..
            }
        )));
        assert!(authored.rule.predicates().iter().any(|predicate| matches!(predicate, crate::arena::conditions::RulePredicate::Odds { weapon: Some(weapon), min_permille: 700 } if weapon == "laser")));
        let held = parse("FIRE ALL HOLD WHEN hostiles >= 2", 100).unwrap();
        assert_eq!(
            held.rule.weapon().map(|weapon| weapon.as_str()),
            Some("all")
        );
        assert!(held.rule.predicates().is_empty());
        assert!(parse("LOCK ALL WHEN no lock", 100).is_err());
        assert!(parse("FIRE ALL WHEN odds ALL >= 60", 100).is_err());
    }
    #[test]
    fn custom_canonical_operands_are_independent_of_test_hardware_and_each_other() {
        use crate::arena::conditions::RulePredicate;
        let authored = parse(
            "FIRE Modded_Rail_Cannon WHEN rounds > 50000 AND rounds Aux_Missile < 25% AND odds Aux_Laser>=60",
            100,
        ).unwrap();
        assert_eq!(authored.rule.weapon().unwrap(), "modded_rail_cannon");
        assert!(matches!(
            &authored.rule.predicates()[0],
            RulePredicate::Rounds {
                weapon: None,
                amount: 50000,
                percent: false,
                ..
            }
        ));
        assert!(
            matches!(&authored.rule.predicates()[1], RulePredicate::Rounds {
            weapon: Some(weapon), amount: 25, percent: true, ..
        } if weapon == "aux_missile")
        );
        assert!(
            matches!(&authored.rule.predicates()[2], RulePredicate::Odds {
            weapon: Some(weapon), min_permille: 600
        } if weapon == "aux_laser")
        );
        assert!(parse("LOCK nearest WHEN rounds > 25% AND no lock", 100).is_ok());
        // Prefixes and acronym-like identifiers remain exact symbols; only a
        // subsequent hull binding can decide whether the component is carried.
        assert_eq!(
            parse("FIRE mrc WHEN odds >= 60", 100)
                .unwrap()
                .rule
                .weapon()
                .unwrap(),
            "mrc"
        );
        for invalid in [
            "FIRE x WHEN rounds > 101%",
            "FIRE x WHEN rounds > 4294967296",
            "FIRE x WHEN odds x/y >= 60",
            "FIRE all WHEN rounds all > 25%",
            "FIRE __authored_default WHEN no lock",
        ] {
            assert!(parse(invalid, 100).is_err(), "{invalid}");
        }
    }

    #[test]
    fn think_terms_gate_the_complete_shared_action() {
        let result = parse(
            "FIRE cannon WHEN hit within 2s AND hostiles>=2 AND odds >= 60 AND order=hold",
            100,
        )
        .unwrap();
        assert_eq!(result.think.len(), 3);
        assert_eq!(result.rule.predicates().len(), 1);
        assert_eq!(result.rule.fire().unwrap().min_permille, 600);
        let text = words(&result);
        assert!(
            text.contains("at least 60%")
                && text.contains("within 2000 ms")
                && text.contains("at least 2")
                && text.contains("order is hold")
        );
        let result = parse("FIRE cannon HOLD WHEN hit within 100ms", 100).unwrap();
        assert!(result.rule.predicates().is_empty());
        assert_eq!(result.think.len(), 1);
        for text in [
            "FIRE cannon WHEN hostiles >= 0",
            "FIRE cannon WHEN hit within 301s",
            "FIRE cannon WHEN order = secret",
            "FIRE cannon WHEN target = A1",
            "FIRE cannon WHEN hostiles >= 2 AND hostiles >= 3",
        ] {
            assert!(parse(text, 100).is_err(), "{text}");
        }
    }
    #[test]
    fn m6_editable_rule_source_round_trips_every_author_grammar_variant() {
        use crate::arena::schema::rule_source;
        let cases = [
            "FIRE cannon WHEN odds>=60",
            "FIRE all WHEN rounds>25% AND odds laser>=70",
            "FIRE cannon HOLD WHEN heat>80%",
            "FIRE all HOLD WHEN hostiles>=2",
            "FIRE cannon WHEN target=lock",
            "FIRE cannon WHEN target=best_hostile AND range<900m",
            "FIRE cannon WHEN aspect=front,side,rear AND range>25m AND rounds aux<20 AND reserve",
            "FIRE cannon WHEN band=close AND target=identified",
            "FIRE cannon WHEN band=effective AND target=moving",
            "FIRE cannon WHEN band=far AND target=stopped",
            "FIRE cannon WHEN target=hurt AND target turret=green",
            "FIRE cannon WHEN target turret=yellow AND target engine=red AND target legs=destroyed",
            "FIRE cannon WHEN own turret<20% AND heat>90% AND no lock",
            "FIRE cannon HOLD WHEN lock AND rounds aux>30% AND odds cannon>=50",
            "LOCK nearest WHEN no lock",
            "LOCK best_odds WHEN odds laser>=70 AND band=effective",
            "LOCK weakest WHEN target=hurt",
            "LOCK flank WHEN aspect=side,rear",
            "LOCK newest WHEN hit within 2000ms AND hostiles>=2 AND order=hold",
            "FIRE all HOLD WHEN order=none",
            "FIRE all HOLD WHEN order=advance",
            "FIRE all HOLD WHEN order=engage",
            "FIRE all HOLD WHEN order=withdraw",
            "FIRE all HOLD WHEN order=screen",
            "FIRE all HOLD WHEN order=support",
        ];
        for source in cases {
            let rule = parse(source, 100).expect(source);
            let rendered = rule_source(&rule).expect(source);
            assert_eq!(
                parse(&rendered, 100).expect(&rendered),
                rule,
                "{source} -> {rendered}"
            );
        }
    }

    #[test]
    fn m6_editable_rule_source_refuses_structured_only_forms_without_lossy_rewrite() {
        use crate::arena::conditions::{Comparison, LockSelector, RulePredicate};
        use crate::arena::schema::rule_source;
        let mut rule = AuthoredRule {
            rule: Rule::hold_rule("cannon".into(), vec![]),
            think: vec![],
        };
        assert!(
            rule_source(&rule).is_err(),
            "unconditional HOLD has no text spelling"
        );
        rule.rule = Rule::lock_rule(LockSelector::Nearest, None, vec![]);
        assert!(rule_source(&rule).is_err());
        for predicate in [
            RulePredicate::Aspect { mask: 0b0010 },
            RulePredicate::Odds {
                weapon: None,
                min_permille: 505,
            },
            RulePredicate::Range {
                comparison: Comparison::AtLeast,
                metres: 50,
            },
            RulePredicate::Rounds {
                weapon: None,
                comparison: Comparison::Equal,
                amount: 5,
                percent: false,
            },
        ] {
            rule.rule = Rule::hold_rule("cannon".into(), vec![predicate]);
            assert!(rule_source(&rule).is_err());
        }
    }
}
