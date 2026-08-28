//! Data-driven costs and pools (crib §CHARGEN / §XP). Everything a house rule
//! or a splat sub-module might tune lives here, not baked into logic — the
//! `WodRuleset` carries one [`WodConfig`] and the chargen/XP code reads it.
//!
//! Defaults are the Classic World of Darkness 20th Anniversary values from the
//! milestone crib exactly:
//!
//! - Attributes 7/5/3 priority dots (on top of the free 1 each).
//! - Abilities 13/9/5 priority dots (cap 3 during initial allocation).
//! - Backgrounds 5 dots.
//! - 15 freebie points: Attribute 5/dot, Ability 2/dot, Background 1/dot,
//!   Willpower 1/dot, Virtue 2/dot.
//! - XP: current-rating × multiplier (Attribute ×4, Ability ×2, Background ×3,
//!   Willpower ×1, Virtue ×2); a brand-new Ability dot costs a flat 3.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The three attribute/ability priority tiers, highest first.
pub const PRIORITY_ATTR: [i32; 3] = [7, 5, 3];
/// Ability priority tiers, highest first.
pub const PRIORITY_ABIL: [i32; 3] = [13, 9, 5];

/// A trait "kind" for costing (chargen freebies + XP). Derived from the sheet
/// field's section/category so the cost tables need no per-field entries.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraitKind {
    /// One of the nine attributes.
    Attribute,
    /// A Talent/Skill/Knowledge ability.
    Ability,
    /// A Background (map entry).
    Background,
    /// Permanent Willpower.
    Willpower,
    /// A Virtue (Conscience/Self-Control/Courage).
    Virtue,
}

/// Freebie-point costs per dot, by trait kind (crib chargen table).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreebieCosts {
    /// Cost per attribute dot bought with freebies.
    pub attribute: i32,
    /// Cost per ability dot.
    pub ability: i32,
    /// Cost per background dot.
    pub background: i32,
    /// Cost per willpower dot.
    pub willpower: i32,
    /// Cost per virtue dot.
    pub virtue: i32,
}

impl Default for FreebieCosts {
    fn default() -> FreebieCosts {
        FreebieCosts {
            attribute: 5,
            ability: 2,
            background: 1,
            willpower: 1,
            virtue: 2,
        }
    }
}

impl FreebieCosts {
    /// The freebie cost per dot for `kind`.
    #[must_use]
    pub fn per_dot(&self, kind: TraitKind) -> i32 {
        match kind {
            TraitKind::Attribute => self.attribute,
            TraitKind::Ability => self.ability,
            TraitKind::Background => self.background,
            TraitKind::Willpower => self.willpower,
            TraitKind::Virtue => self.virtue,
        }
    }
}

/// XP cost rule for a trait kind: `cost(from→from+1) = max(from, 1) × mult`
/// (crib: current-rating × multiplier), except a first dot bought from 0 costs
/// `flat_new` when set (the "new Ability = 3" rule).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XpRule {
    /// Per-step multiplier applied to the current rating.
    pub mult: i32,
    /// Flat cost of the very first dot (0 → 1); `None` uses `mult` with a
    /// current rating treated as 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flat_new: Option<i32>,
}

impl XpRule {
    /// A rule with a per-step multiplier and no special new-dot cost.
    #[must_use]
    pub fn mult(mult: i32) -> XpRule {
        XpRule {
            mult,
            flat_new: None,
        }
    }

    /// A rule with a per-step multiplier and a flat first-dot cost.
    #[must_use]
    pub fn with_new(mult: i32, flat_new: i32) -> XpRule {
        XpRule {
            mult,
            flat_new: Some(flat_new),
        }
    }

    /// The cost of the single dot that takes a trait from `from` to `from + 1`.
    #[must_use]
    pub fn step_cost(&self, from: i32) -> i32 {
        if from <= 0 {
            self.flat_new.unwrap_or(self.mult)
        } else {
            from * self.mult
        }
    }

    /// The total cost to raise a trait from `from` to `to` (`0` if `to <= from`).
    #[must_use]
    pub fn raise_cost(&self, from: i32, to: i32) -> i32 {
        (from.max(0)..to).map(|r| self.step_cost(r)).sum()
    }
}

/// The complete XP multiplier table (crib §XP).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XpCosts {
    /// Attribute: current × 4.
    pub attribute: XpRule,
    /// Ability: current × 2, new dot = 3.
    pub ability: XpRule,
    /// Background: current × 3.
    pub background: XpRule,
    /// Willpower: current × 1.
    pub willpower: XpRule,
    /// Virtue: current × 2.
    pub virtue: XpRule,
}

impl Default for XpCosts {
    fn default() -> XpCosts {
        XpCosts {
            attribute: XpRule::mult(4),
            ability: XpRule::with_new(2, 3),
            background: XpRule::mult(3),
            willpower: XpRule::mult(1),
            virtue: XpRule::mult(2),
        }
    }
}

impl XpCosts {
    /// The rule for `kind`.
    #[must_use]
    pub fn rule(&self, kind: TraitKind) -> &XpRule {
        match kind {
            TraitKind::Attribute => &self.attribute,
            TraitKind::Ability => &self.ability,
            TraitKind::Background => &self.background,
            TraitKind::Willpower => &self.willpower,
            TraitKind::Virtue => &self.virtue,
        }
    }
}

/// How a fresh character's starting **permanent** Willpower is derived at
/// chargen finalize. The V20/Vampire convention is "Willpower = Courage", but
/// the baseline is splat-specific (Mage starts at 5, Werewolf by Auspice), so
/// it is data — not baked into the chargen logic. It also fixes the freebie
/// baseline: Willpower dots above it cost freebies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartingWillpower {
    /// Equal to a virtue's rating (V20 Vampire/Mortal: Courage).
    FromVirtue(SmolStr),
    /// A flat starting rating (e.g. Mage 5).
    Flat(i32),
}

impl Default for StartingWillpower {
    fn default() -> StartingWillpower {
        StartingWillpower::FromVirtue(SmolStr::new("courage"))
    }
}

/// The full tunable ruleset config (chargen + XP). Splats / house rules build
/// a modified one and hand it to [`WodRuleset`](crate::WodRuleset).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WodConfig {
    /// Attribute priority tiers (dots above the free 1), highest first.
    pub attr_priorities: [i32; 3],
    /// Ability priority tiers, highest first.
    pub abil_priorities: [i32; 3],
    /// Background dots at chargen.
    pub background_dots: i32,
    /// Freebie points at chargen.
    pub freebies: i32,
    /// Per-ability cap during initial (priority) allocation; freebies may buy
    /// higher.
    pub ability_initial_cap: i32,
    /// Normal trait maximum (attributes/abilities cap here overall).
    pub normal_max: i32,
    /// Freebie costs per dot.
    pub freebie_costs: FreebieCosts,
    /// XP multiplier table.
    pub xp_costs: XpCosts,
    /// How starting permanent Willpower is derived at chargen finalize (and
    /// the baseline above which Willpower dots cost freebies).
    #[serde(default)]
    pub starting_willpower: StartingWillpower,
}

impl Default for WodConfig {
    fn default() -> WodConfig {
        WodConfig {
            attr_priorities: PRIORITY_ATTR,
            abil_priorities: PRIORITY_ABIL,
            background_dots: 5,
            freebies: 15,
            ability_initial_cap: 3,
            normal_max: 5,
            freebie_costs: FreebieCosts::default(),
            xp_costs: XpCosts::default(),
            starting_willpower: StartingWillpower::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xp_step_and_raise() {
        // Attribute ×4: 2→3 costs 2×4 = 8; 1→3 costs 4 + 8 = 12.
        let a = XpRule::mult(4);
        assert_eq!(a.step_cost(2), 8);
        assert_eq!(a.raise_cost(1, 3), 12);
        // Ability ×2 with new=3: 0→1 = 3, 0→2 = 3 + (1×2) = 5.
        let ab = XpRule::with_new(2, 3);
        assert_eq!(ab.step_cost(0), 3);
        assert_eq!(ab.raise_cost(0, 2), 5);
        // Willpower ×1: 5→7 = 5 + 6 = 11.
        assert_eq!(XpRule::mult(1).raise_cost(5, 7), 11);
    }

    #[test]
    fn defaults_match_crib() {
        let c = WodConfig::default();
        assert_eq!(c.attr_priorities, [7, 5, 3]);
        assert_eq!(c.abil_priorities, [13, 9, 5]);
        assert_eq!(c.freebies, 15);
        assert_eq!(c.freebie_costs.attribute, 5);
        assert_eq!(c.ability_initial_cap, 3);
        assert_eq!(c.xp_costs.rule(TraitKind::Attribute).mult, 4);
        // V20 default: starting Willpower derives from Courage.
        assert_eq!(
            c.starting_willpower,
            StartingWillpower::FromVirtue(SmolStr::new("courage"))
        );
    }

    #[test]
    fn config_round_trips_through_json_with_defaulted_willpower() {
        // A config serialized before `starting_willpower` existed still
        // deserializes (the field is `#[serde(default)]`).
        let mut v = serde_json::to_value(WodConfig::default()).unwrap();
        v.as_object_mut().unwrap().remove("starting_willpower");
        let c: WodConfig = serde_json::from_value(v).unwrap();
        assert_eq!(c.starting_willpower, StartingWillpower::default());
    }
}
