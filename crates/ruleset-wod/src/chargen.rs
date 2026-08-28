//! oWoD chargen accounting and the [`ChargenFlow`] implementation.
//!
//! The classic priority point-buy from the crib: Attributes 7/5/3 across
//! Physical/Social/Mental, Abilities 13/9/5 across Talents/Skills/Knowledges
//! (cap 3 during initial allocation), Backgrounds 5, then 15 freebie points.
//!
//! **Freebie model** (clean and rules-faithful): freebies are exactly the dots
//! spent *beyond* the priority pools. A category that spends more dots than its
//! priority pool holds pays `cost/dot` freebies for the overflow; ability dots
//! above the initial cap of 3 are freebie-only; willpower above the Courage
//! baseline and every virtue dot above 1 are freebies too. One source of truth
//! — the sheet dots — so there is no "which dot came from where" bookkeeping.
//!
//! The pool totals come from the player's priority assignment (held in the
//! [`ChargenSession`](crate::state::ChargenSession)); [`WodChargen`] uses the
//! config's default ordering as the framework-facing reference flow.

use std::sync::Arc;

use mush_ruleset::SheetDoc;
use mush_ruleset::chargen::{BudgetLedger, ChargenFlow, ChargenIssue, ChargenStage};
use smol_str::SmolStr;

use crate::config::{TraitKind, WodConfig};
use crate::schema::{ATTRIBUTES, KNOWLEDGES, SKILLS, TALENTS, VIRTUES};
use crate::splats::{Dispatch, Splat, SplatPool};
use crate::state::ChargenSession;

/// The concrete pool totals for one chargen assessment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pools {
    /// Attribute pools: `[(category, dots), …]`.
    pub attr: Vec<(SmolStr, i32)>,
    /// Ability pools: `[(group, dots), …]`.
    pub abil: Vec<(SmolStr, i32)>,
    /// Background dots.
    pub backgrounds: i32,
    /// Freebie points.
    pub freebies: i32,
    /// The active splat's extra pools (Vampire: Disciplines, Virtues). Empty
    /// for a mortal.
    pub splat: Vec<SplatPool>,
    /// Base trait kinds the splat pools already account for, so base freebie
    /// counting skips them (Vampire: [`TraitKind::Virtue`]).
    pub freebie_exempt: Vec<TraitKind>,
}

impl Pools {
    /// Attach the active splat's chargen contribution (pools + freebie
    /// exemptions), computed against the in-progress `doc`.
    #[must_use]
    pub(crate) fn with_splat(
        mut self,
        config: &WodConfig,
        splat: &dyn Splat,
        doc: &SheetDoc,
    ) -> Pools {
        let c = splat.chargen_contribution(config, doc);
        self.splat = c.pools;
        self.freebie_exempt = c.freebie_exempt;
        self
    }
}

/// The default pools from config, assigning the priority tiers to
/// Physical/Social/Mental and Talents/Skills/Knowledges in declaration order
/// (the framework-facing reference; the live UX uses the player's choice).
#[must_use]
pub fn default_pools(config: &WodConfig) -> Pools {
    Pools {
        attr: vec![
            (SmolStr::new("physical"), config.attr_priorities[0]),
            (SmolStr::new("social"), config.attr_priorities[1]),
            (SmolStr::new("mental"), config.attr_priorities[2]),
        ],
        abil: vec![
            (SmolStr::new("talents"), config.abil_priorities[0]),
            (SmolStr::new("skills"), config.abil_priorities[1]),
            (SmolStr::new("knowledges"), config.abil_priorities[2]),
        ],
        backgrounds: config.background_dots,
        freebies: config.freebies,
        splat: Vec::new(),
        freebie_exempt: Vec::new(),
    }
}

/// The pools for a live chargen session (falls back to the config defaults for
/// any priority set the player has not chosen yet).
#[must_use]
pub fn session_pools(config: &WodConfig, session: &ChargenSession) -> Pools {
    let mut pools = default_pools(config);
    if !session.attr_pools.is_empty() {
        pools.attr = session.attr_pools.clone();
    }
    if !session.abil_pools.is_empty() {
        pools.abil = session.abil_pools.clone();
    }
    pools
}

/// The staff-only sheet field recording the priority assignment the sheet was
/// built against. Persisting it makes chargen reboot-safe (a lost session
/// re-hydrates from here rather than silently re-baselining to the defaults)
/// and lets staff approval re-validate the budget against the real pools.
pub const CHARGEN_POOLS_FIELD: &str = "chargen_pools";

/// The attribute category ids, in priority order.
pub const ATTR_CATS: [&str; 3] = ["physical", "social", "mental"];
/// The ability group ids, in priority order.
pub const ABIL_GROUPS: [&str; 3] = ["talents", "skills", "knowledges"];

/// One priority assignment: `[(category/group, tier dots), …]`.
pub type PoolAssignment = Vec<(SmolStr, i32)>;

/// Read a persisted priority assignment for `keys` from the sheet's
/// `chargen_pools` map, or `None` if any key is missing.
fn persisted_assignment(doc: &SheetDoc, keys: &[&str]) -> Option<PoolAssignment> {
    let m = doc.map(CHARGEN_POOLS_FIELD)?;
    let mut out = Vec::with_capacity(keys.len());
    for k in keys {
        let v = m.get(*k)?.as_i64()? as i32;
        out.push((SmolStr::new(*k), v));
    }
    Some(out)
}

/// The attribute + ability priority assignment persisted on the sheet (empty
/// vecs when nothing is recorded yet) — used to re-hydrate a session on resume.
#[must_use]
pub fn persisted_priorities(doc: &SheetDoc) -> (PoolAssignment, PoolAssignment) {
    (
        persisted_assignment(doc, &ATTR_CATS).unwrap_or_default(),
        persisted_assignment(doc, &ABIL_GROUPS).unwrap_or_default(),
    )
}

/// The pools the sheet was built against: the persisted priority assignment
/// (falling back to the config defaults for any set not yet chosen). This is
/// the reboot-safe / approval-time source of truth, independent of the
/// transient session.
#[must_use]
pub fn persisted_pools(config: &WodConfig, doc: &SheetDoc) -> Pools {
    let mut pools = default_pools(config);
    if let Some(attr) = persisted_assignment(doc, &ATTR_CATS) {
        pools.attr = attr;
    }
    if let Some(abil) = persisted_assignment(doc, &ABIL_GROUPS) {
        pools.abil = abil;
    }
    pools
}

/// Attribute dots spent above the free 1 in a category.
fn attr_eligible(doc: &SheetDoc, category: &str) -> i32 {
    ATTRIBUTES
        .iter()
        .filter(|(_, _, cat)| *cat == category)
        .map(|(id, _, _)| (doc.number(id).unwrap_or(1) as i32 - 1).max(0))
        .sum()
}

/// The ability ids in a group.
fn group_ability_ids(group: &str) -> &'static [(&'static str, &'static str)] {
    match group {
        "talents" => TALENTS,
        "skills" => SKILLS,
        "knowledges" => KNOWLEDGES,
        _ => &[],
    }
}

/// `(priority_dots, above_cap_dots)` for an ability group: priority dots are
/// capped per-ability at `cap`; the remainder is freebie-only.
fn abil_split(doc: &SheetDoc, group: &str, cap: i32) -> (i32, i32) {
    let mut prio = 0;
    let mut above = 0;
    for (id, _) in group_ability_ids(group) {
        let d = doc.number(id).unwrap_or(0) as i32;
        prio += d.min(cap);
        above += (d - cap).max(0);
    }
    (prio, above)
}

/// Total background dots on the sheet.
fn background_total(doc: &SheetDoc) -> i32 {
    doc.map("backgrounds")
        .map(|m| {
            m.values()
                .filter_map(serde_json::Value::as_i64)
                .sum::<i64>() as i32
        })
        .unwrap_or(0)
}

/// The starting-Willpower baseline (config-driven; the V20 default is
/// Courage). Willpower dots above it cost freebies, and finalize floors
/// permanent Willpower at it.
fn starting_wp_baseline(config: &WodConfig, doc: &SheetDoc) -> i32 {
    match &config.starting_willpower {
        crate::config::StartingWillpower::FromVirtue(v) => doc.number(v).unwrap_or(1) as i32,
        crate::config::StartingWillpower::Flat(n) => *n,
    }
}

/// The total freebie points spent (crib freebie model).
#[must_use]
pub fn freebie_spent(config: &WodConfig, doc: &SheetDoc, pools: &Pools) -> i32 {
    let costs = &config.freebie_costs;
    let mut f = 0;
    for (cat, total) in &pools.attr {
        let elig = attr_eligible(doc, cat);
        f += (elig - total).max(0) * costs.attribute;
    }
    for (grp, total) in &pools.abil {
        let (prio, above_cap) = abil_split(doc, grp, config.ability_initial_cap);
        f += (prio - total).max(0) * costs.ability;
        f += above_cap * costs.ability;
    }
    let bg = background_total(doc);
    f += (bg - pools.backgrounds).max(0) * costs.background;
    // Base Willpower freebies — skipped when the active splat derives Willpower
    // to a fixed baseline rather than the config's (Mage: a flat 5, exempt so
    // its free dots are not charged against the Courage baseline).
    if !pools.freebie_exempt.contains(&TraitKind::Willpower) {
        let wp = doc.number("willpower").unwrap_or(1) as i32;
        f += (wp - starting_wp_baseline(config, doc)).max(0) * costs.willpower;
    }
    // Base Virtue freebies — skipped when the active splat's own Virtue pool
    // already accounts for them (Vampire).
    if !pools.freebie_exempt.contains(&TraitKind::Virtue) {
        for (id, _) in VIRTUES {
            let d = doc.number(id).unwrap_or(1) as i32;
            f += (d - 1).max(0) * costs.virtue;
        }
    }
    // Splat pools: dots spent beyond the pool total cost freebies.
    for pool in &pools.splat {
        f += (pool.spent(doc) - pool.total).max(0) * pool.freebie_cost;
    }
    f
}

/// Compute every budget ledger for a chargen state.
#[must_use]
pub fn ledgers(config: &WodConfig, doc: &SheetDoc, pools: &Pools) -> Vec<BudgetLedger> {
    let mut out = Vec::new();
    for (cat, total) in &pools.attr {
        out.push(BudgetLedger {
            name: cat.clone(),
            label: cap_first(cat),
            total: *total,
            spent: attr_eligible(doc, cat),
        });
    }
    for (grp, total) in &pools.abil {
        let (prio, _) = abil_split(doc, grp, config.ability_initial_cap);
        out.push(BudgetLedger {
            name: grp.clone(),
            label: cap_first(grp),
            total: *total,
            spent: prio,
        });
    }
    out.push(BudgetLedger {
        name: SmolStr::new("backgrounds"),
        label: "Backgrounds".to_string(),
        total: pools.backgrounds,
        spent: background_total(doc),
    });
    // Splat pools (Disciplines, Virtues) sit between backgrounds and freebies.
    for pool in &pools.splat {
        out.push(BudgetLedger {
            name: pool.id.clone(),
            label: pool.label.clone(),
            total: pool.total,
            spent: pool.spent(doc),
        });
    }
    out.push(BudgetLedger {
        name: SmolStr::new("freebies"),
        label: "Freebies".to_string(),
        total: pools.freebies,
        spent: freebie_spent(config, doc, pools),
    });
    out
}

/// Chargen issues: priority pools must be fully spent (overflow into freebies
/// is fine, not an overspend); freebies must not exceed the pool.
#[must_use]
pub fn issues(config: &WodConfig, doc: &SheetDoc, pools: &Pools) -> Vec<ChargenIssue> {
    let mut issues = Vec::new();
    for l in ledgers(config, doc, pools) {
        if l.name == "freebies" {
            if l.spent > l.total {
                issues.push(ChargenIssue::Overspent {
                    pool: l.name.clone(),
                    label: l.label.clone(),
                    by: l.spent - l.total,
                });
            }
        } else if l.spent < l.total {
            issues.push(ChargenIssue::Underspent {
                pool: l.name.clone(),
                label: l.label.clone(),
                by: l.total - l.spent,
            });
        }
    }
    issues
}

/// Finalize: verify the budget, derive Willpower (max of current and the
/// config starting-Willpower baseline, Courage by default), set the temporary
/// Willpower pool to full, then let the active `splat` derive its own starting
/// values (Vampire: Blood Pool full, Humanity, clan weakness).
///
/// # Errors
///
/// Returns the first blocking [`ChargenIssue`] if the budget does not balance.
pub(crate) fn finalize_sheet(
    config: &WodConfig,
    doc: &mut SheetDoc,
    pools: &Pools,
    splat: &dyn Splat,
) -> Result<(), ChargenIssue> {
    if let Some(first) = issues(config, doc, pools).into_iter().next() {
        return Err(first);
    }
    let wp = (doc.number("willpower").unwrap_or(1) as i32).max(starting_wp_baseline(config, doc));
    doc.set_number("willpower", i64::from(wp));
    doc.set_number("willpower_temp", i64::from(wp));
    splat.finalize_starting(config, doc);
    Ok(())
}

fn cap_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// The oWoD chargen flow (framework-facing). Assesses against the priority
/// assignment persisted on the sheet (the player's choice) **plus** the active
/// template's chargen contribution, matching the live `+chargen` command
/// surface, and falls back to the config defaults only when no assignment is
/// recorded.
pub struct WodChargen {
    dispatch: Arc<Dispatch>,
    config: WodConfig,
}

impl WodChargen {
    /// A flow over the multi-template `dispatch` with `config`.
    #[must_use]
    pub(crate) fn new(dispatch: Arc<Dispatch>, config: WodConfig) -> WodChargen {
        WodChargen { dispatch, config }
    }

    /// The config this flow costs against.
    #[must_use]
    pub fn config(&self) -> &WodConfig {
        &self.config
    }

    /// The pools for a sheet: persisted priority pools + the active splat's
    /// contribution (selected by the sheet's `template`).
    fn pools_for(&self, sheet: &SheetDoc) -> Pools {
        let splat = self.dispatch.splats.for_doc(sheet);
        persisted_pools(&self.config, sheet).with_splat(&self.config, splat.as_ref(), sheet)
    }
}

impl ChargenFlow for WodChargen {
    fn stages(&self) -> Vec<ChargenStage> {
        vec![
            ChargenStage::new(
                "priorities",
                "Priorities",
                "Assign 7/5/3 to Physical/Social/Mental and 13/9/5 to \
                 Talents/Skills/Knowledges with +chargen/attr and +chargen/abil.",
            ),
            ChargenStage::new(
                "attributes",
                "Attributes",
                "Raise your nine attributes with +stat (all start at 1).",
            ),
            ChargenStage::new(
                "abilities",
                "Abilities",
                "Buy abilities with +stat (cap 3 before freebies).",
            ),
            ChargenStage::new(
                "backgrounds",
                "Backgrounds",
                "Spend 5 background dots: +stat backgrounds:<name>=<dots>.",
            ),
            ChargenStage::new(
                "freebies",
                "Freebies",
                "Spend up to 15 freebie points on anything beyond your pools.",
            ),
            ChargenStage::new(
                "concept",
                "Concept",
                "Set concept/nature/demeanor with +stat, then +chargen/submit.",
            ),
        ]
    }

    fn validate_stage(&self, sheet: &SheetDoc, stage: &SmolStr) -> Vec<ChargenIssue> {
        // Assess against the priority assignment persisted on the sheet (the
        // player's real choice) plus the active splat's pools, falling back to
        // the config defaults only when none is recorded — so this
        // framework-facing flow matches the live `+chargen` command surface.
        let pools = self.pools_for(sheet);
        let all = issues(&self.config, sheet, &pools);
        // Filter to the pools relevant to the named stage.
        all.into_iter()
            .filter(|i| stage_owns(stage, issue_pool(i)))
            .collect()
    }

    fn finalize(&self, sheet: &mut SheetDoc) -> Result<(), ChargenIssue> {
        let pools = self.pools_for(sheet);
        let splat = self.dispatch.splats.for_doc(sheet);
        finalize_sheet(&self.config, sheet, &pools, splat.as_ref())
    }
}

/// The pool id an issue is about.
fn issue_pool(i: &ChargenIssue) -> &str {
    match i {
        ChargenIssue::Overspent { pool, .. } | ChargenIssue::Underspent { pool, .. } => {
            pool.as_str()
        }
        ChargenIssue::AboveCap { field, .. } => field.as_str(),
        _ => "",
    }
}

/// Whether `stage` owns `pool` (for per-stage validation filtering).
fn stage_owns(stage: &str, pool: &str) -> bool {
    match stage {
        "attributes" => matches!(pool, "physical" | "social" | "mental"),
        "abilities" => matches!(pool, "talents" | "skills" | "knowledges"),
        "backgrounds" => pool == "backgrounds",
        "freebies" => pool == "freebies",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use mush_ruleset::schema::SheetSchema;

    use super::*;
    use crate::schema::{Mortal, Template, build_schema};

    fn schema() -> Arc<SheetSchema> {
        Arc::new(build_schema(
            "wod",
            &[Box::new(Mortal) as Box<dyn Template>],
        ))
    }

    /// Spend the physical pool exactly: 7 dots (str/dex/sta = 4/3/2 → 3+2+1=6…),
    /// use 4/4/2 → (3)+(3)+(1)=7.
    fn spend_physical(doc: &mut SheetDoc) {
        doc.set_number("strength", 4);
        doc.set_number("dexterity", 4);
        doc.set_number("stamina", 2);
    }

    #[test]
    fn physical_pool_exact_spend() {
        let s = schema();
        let cfg = WodConfig::default();
        let pools = default_pools(&cfg);
        let mut doc = SheetDoc::new_default(&s);
        // 1/1/1 → 0 spent, 7 unspent.
        let l = ledgers(&cfg, &doc, &pools);
        let phys = l.iter().find(|l| l.name == "physical").unwrap();
        assert_eq!(phys.spent, 0);
        assert_eq!(phys.total, 7);
        spend_physical(&mut doc);
        let l = ledgers(&cfg, &doc, &pools);
        assert_eq!(l.iter().find(|l| l.name == "physical").unwrap().spent, 7);
    }

    #[test]
    fn overflow_beyond_pool_costs_freebies() {
        let s = schema();
        let cfg = WodConfig::default();
        let pools = default_pools(&cfg);
        let mut doc = SheetDoc::new_default(&s);
        // Physical pool 7; spend 9 (str5 dex5 sta1 → 4+4+0 = 8; make sta2 → 9).
        doc.set_number("strength", 5);
        doc.set_number("dexterity", 5);
        doc.set_number("stamina", 2);
        // eligible physical = 4+4+1 = 9; overflow 2 → 2*5 = 10 freebies.
        assert_eq!(freebie_spent(&cfg, &doc, &pools), 10);
    }

    #[test]
    fn ability_above_cap_is_freebie_priced() {
        let s = schema();
        let cfg = WodConfig::default();
        let pools = default_pools(&cfg);
        let mut doc = SheetDoc::new_default(&s);
        // Brawl (talent) at 5: priority dots capped at 3, 2 above cap.
        doc.set_number("brawl", 5);
        let (prio, above) = abil_split(&doc, "talents", cfg.ability_initial_cap);
        assert_eq!(prio, 3);
        assert_eq!(above, 2);
        // 2 above-cap ability dots → 2*2 = 4 freebies (talents pool still unspent).
        assert_eq!(freebie_spent(&cfg, &doc, &pools), 4);
    }

    #[test]
    fn persisted_pools_read_back_or_fall_back() {
        let s = schema();
        let cfg = WodConfig::default();
        let mut doc = SheetDoc::new_default(&s);
        // No chargen_pools recorded yet → the config defaults.
        let p = persisted_pools(&cfg, &doc);
        assert_eq!(p.attr, default_pools(&cfg).attr);
        // Record a non-default assignment; persisted_pools reads it back.
        for (k, v) in [
            ("physical", 3),
            ("social", 7),
            ("mental", 5),
            ("talents", 5),
            ("skills", 9),
            ("knowledges", 13),
        ] {
            doc.map_set(CHARGEN_POOLS_FIELD, k, serde_json::json!(v));
        }
        let p = persisted_pools(&cfg, &doc);
        assert_eq!(
            p.attr,
            vec![
                (SmolStr::new("physical"), 3),
                (SmolStr::new("social"), 7),
                (SmolStr::new("mental"), 5),
            ]
        );
        assert_eq!(
            p.abil,
            vec![
                (SmolStr::new("talents"), 5),
                (SmolStr::new("skills"), 9),
                (SmolStr::new("knowledges"), 13),
            ]
        );
        let (attr, abil) = persisted_priorities(&doc);
        assert_eq!(attr.len(), 3);
        assert_eq!(abil.len(), 3);
    }

    #[test]
    fn starting_willpower_is_configurable() {
        use crate::config::StartingWillpower;
        let s = schema();
        // A flat baseline (Mage-style WP 5): finalize floors WP at 5, and the
        // first five WP dots cost no freebies.
        let cfg = WodConfig {
            starting_willpower: StartingWillpower::Flat(5),
            ..WodConfig::default()
        };
        let pools = Pools {
            attr: vec![
                (SmolStr::new("physical"), 0),
                (SmolStr::new("social"), 0),
                (SmolStr::new("mental"), 0),
            ],
            abil: vec![
                (SmolStr::new("talents"), 0),
                (SmolStr::new("skills"), 0),
                (SmolStr::new("knowledges"), 0),
            ],
            backgrounds: 0,
            freebies: 15,
            splat: Vec::new(),
            freebie_exempt: Vec::new(),
        };
        let mut doc = SheetDoc::new_default(&s);
        // WP left at the schema default (1) → finalize raises it to the flat 5.
        assert_eq!(freebie_spent(&cfg, &doc, &pools), 0);
        finalize_sheet(&cfg, &mut doc, &pools, &crate::splats::MortalSplat).unwrap();
        assert_eq!(doc.number("willpower"), Some(5));
        assert_eq!(doc.number("willpower_temp"), Some(5));
    }

    #[test]
    fn finalize_derives_willpower_from_courage() {
        let s = schema();
        let cfg = WodConfig::default();
        // Zero out priority requirements so the budget balances trivially.
        let pools = Pools {
            attr: vec![
                (SmolStr::new("physical"), 0),
                (SmolStr::new("social"), 0),
                (SmolStr::new("mental"), 0),
            ],
            abil: vec![
                (SmolStr::new("talents"), 0),
                (SmolStr::new("skills"), 0),
                (SmolStr::new("knowledges"), 0),
            ],
            backgrounds: 0,
            freebies: 15,
            splat: Vec::new(),
            freebie_exempt: Vec::new(),
        };
        let mut doc = SheetDoc::new_default(&s);
        doc.set_number("courage", 4);
        finalize_sheet(&cfg, &mut doc, &pools, &crate::splats::MortalSplat).unwrap();
        assert_eq!(doc.number("willpower"), Some(4));
        assert_eq!(doc.number("willpower_temp"), Some(4));
    }
}
