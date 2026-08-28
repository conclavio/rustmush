//! The chargen framework (design doc 04 §3.5): the [`ChargenFlow`] trait, an
//! ordered stage machine, and a generic **point-budget accountant**.
//!
//! Division of labour, per the assignment: **the framework provides the
//! state machine + point-budget accounting; the ruleset provides the
//! stages/costs**. A ruleset declares [`ChargenStage`]s (each with an
//! optional [`BudgetPlan`]) and implements the three [`ChargenFlow`] methods;
//! the framework drives progression ([`ChargenProgress`] / [`try_advance`])
//! and computes over/under-spend and cap issues from a plan
//! ([`BudgetPlan::assess`]).
//!
//! ## Driver contract (stubbed for the WoD agent to implement)
//!
//! The concrete terminal/web driver is the WoD ruleset's job. The
//! **recommended** contract, which the framework's pieces support:
//!
//! - Push a [`mush_core::InputContext`] (the input-context stack — the same
//!   mechanism as mail compose) tagged with the ruleset's module identity,
//!   holding a [`ChargenProgress`] and reading/writing the character's
//!   *Unapproved* `SheetDoc` through [`SheetStore`](crate::store::SheetStore)
//!   each line.
//! - Each stage renders its `prompt`; the player's edits go through the
//!   store's validated setters; a "done" line calls [`try_advance`], which
//!   refuses to move on while [`ChargenFlow::validate_stage`] reports issues.
//! - On the final stage, [`finalize`] computes derived stats and audits the
//!   freebie spend, then the driver flips the sheet to
//!   [`ApprovalState::Pending`](crate::doc::ApprovalState::Pending) and opens
//!   a chargen approval job (via `mush-jobs`, which this crate does not
//!   depend on).
//!
//! Alternatively the flow can be driven as a `+chargen` command set over the
//! same helpers. Either way the state machine and budgeting below are the
//! reusable core.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::doc::SheetDoc;
use crate::schema::{FieldType, SheetSchema};
use crate::validate::SheetError;

/// A guided character-generation flow. Implemented by a ruleset; the
/// framework drives it.
pub trait ChargenFlow: Send + Sync {
    /// The ordered stages (each has prompts + an optional point budget).
    fn stages(&self) -> Vec<ChargenStage>;

    /// Validate one stage against the in-progress sheet, returning every
    /// issue (empty = the stage is complete and consistent). Typically
    /// delegates to [`ChargenStage::assess`] plus ruleset-specific checks.
    fn validate_stage(&self, sheet: &SheetDoc, stage: &SmolStr) -> Vec<ChargenIssue>;

    /// Finalize: compute derived stats, audit the freebie spend, and leave
    /// the sheet ready for approval. Returns the first blocking issue, if
    /// any.
    ///
    /// # Errors
    ///
    /// Returns a [`ChargenIssue`] if the sheet cannot be finalized (e.g. an
    /// unspent or overspent budget).
    fn finalize(&self, sheet: &mut SheetDoc) -> Result<(), ChargenIssue>;
}

/// One ordered chargen stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChargenStage {
    /// Stage id (referenced by [`ChargenFlow::validate_stage`]).
    pub id: SmolStr,
    /// Human label ("Attributes", "Abilities", "Freebies").
    pub label: String,
    /// Prompt text shown while the stage is active.
    pub prompt: String,
    /// The stage's point budget, if it allocates points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetPlan>,
}

impl ChargenStage {
    /// A stage with no budget (pure prompts, e.g. concept/nature).
    #[must_use]
    pub fn new(id: &str, label: impl Into<String>, prompt: impl Into<String>) -> ChargenStage {
        ChargenStage {
            id: SmolStr::new(id),
            label: label.into(),
            prompt: prompt.into(),
            budget: None,
        }
    }

    /// Builder-style: attach a point budget.
    #[must_use]
    pub fn with_budget(mut self, budget: BudgetPlan) -> ChargenStage {
        self.budget = Some(budget);
        self
    }

    /// Run this stage's budget (if any) against the sheet, returning
    /// over/under-spend and cap issues. A convenience for
    /// [`ChargenFlow::validate_stage`].
    #[must_use]
    pub fn assess(&self, schema: &SheetSchema, doc: &SheetDoc) -> Vec<ChargenIssue> {
        self.budget
            .as_ref()
            .map_or_else(Vec::new, |b| b.issues(schema, doc))
    }
}

/// A chargen problem, reported by validation/finalize.
///
/// `Serialize`-only (the `Invalid` variant wraps [`SheetError`]s, which
/// carry non-deserializable `&'static str` fields).
#[derive(Clone, Debug, PartialEq, Serialize, thiserror::Error)]
pub enum ChargenIssue {
    /// A budget pool has more points spent than available.
    #[error("{label}: overspent by {by} point(s)")]
    Overspent {
        /// Pool id.
        pool: SmolStr,
        /// Pool label.
        label: String,
        /// How many points over.
        by: i32,
    },
    /// A budget pool has points left unspent.
    #[error("{label}: {by} point(s) unspent")]
    Underspent {
        /// Pool id.
        pool: SmolStr,
        /// Pool label.
        label: String,
        /// How many points remain.
        by: i32,
    },
    /// A field exceeds its chargen cap (distinct from its schema max — e.g.
    /// abilities cap at 3 during initial allocation).
    #[error("{field}: {got} exceeds the chargen cap of {cap}")]
    AboveCap {
        /// The field id.
        field: SmolStr,
        /// The cap.
        cap: i32,
        /// The value seen.
        got: i32,
    },
    /// The sheet data is invalid (schema validation failed during chargen).
    #[error("sheet is invalid: {0:?}")]
    Invalid(Vec<SheetError>),
    /// A free-form ruleset-specific issue.
    #[error("{0}")]
    Message(String),
}

impl ChargenIssue {
    /// Free-form issue shorthand.
    #[must_use]
    pub fn msg(m: impl Into<String>) -> ChargenIssue {
        ChargenIssue::Message(m.into())
    }
}

// ----------------------------------------------------------------------
// Point-budget accounting
// ----------------------------------------------------------------------

/// One budget pool: a labelled pot of points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PoolDef {
    /// Pool id (referenced by [`CostRule::pool`]).
    pub name: SmolStr,
    /// Human label ("Physical", "Talents", "Freebies").
    pub label: String,
    /// Points available in the pool.
    pub total: i32,
}

impl PoolDef {
    /// A pool.
    #[must_use]
    pub fn new(name: &str, label: impl Into<String>, total: i32) -> PoolDef {
        PoolDef {
            name: SmolStr::new(name),
            label: label.into(),
            total,
        }
    }
}

/// Which fields a [`CostRule`] draws from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldSelector {
    /// Every numeric field with this category.
    Category(SmolStr),
    /// Every numeric field in this section.
    Section(SmolStr),
    /// An explicit field-id list.
    Fields(Vec<SmolStr>),
}

/// A rule mapping dots/points spent on a set of fields onto a pool. Spend
/// for a field is `max(value - base, 0) * cost_per_dot`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostRule {
    /// The pool this rule spends from.
    pub pool: SmolStr,
    /// Which fields it covers.
    pub select: FieldSelector,
    /// Free dots per field (oWoD attributes start at 1, so `base = 1`).
    pub base: i32,
    /// Cost per dot above `base`.
    pub cost_per_dot: i32,
    /// Optional per-field chargen cap (distinct from the schema max).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap: Option<i32>,
}

impl CostRule {
    /// A rule.
    #[must_use]
    pub fn new(pool: &str, select: FieldSelector, base: i32, cost_per_dot: i32) -> CostRule {
        CostRule {
            pool: SmolStr::new(pool),
            select,
            base,
            cost_per_dot,
            cap: None,
        }
    }

    /// Builder-style: set the per-field chargen cap.
    #[must_use]
    pub fn with_cap(mut self, cap: i32) -> CostRule {
        self.cap = Some(cap);
        self
    }

    /// The field ids this rule covers, resolved against the schema (numeric
    /// fields only).
    fn fields<'s>(&self, schema: &'s SheetSchema) -> Vec<&'s SmolStr> {
        let is_numeric =
            |ty: &FieldType| matches!(ty, FieldType::Dots { .. } | FieldType::Number { .. });
        match &self.select {
            FieldSelector::Fields(ids) => ids
                .iter()
                .filter_map(|id| schema.field(id).map(|f| &f.id))
                .collect(),
            FieldSelector::Category(cat) => schema
                .fields()
                .filter(|(_, f)| is_numeric(&f.ty) && f.category.as_ref().is_some_and(|c| c == cat))
                .map(|(_, f)| &f.id)
                .collect(),
            FieldSelector::Section(sec) => schema
                .section(sec)
                .into_iter()
                .flat_map(|s| s.fields.iter())
                .filter(|f| is_numeric(&f.ty))
                .map(|f| &f.id)
                .collect(),
        }
    }
}

/// A running ledger for one pool: total vs. spent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetLedger {
    /// Pool id.
    pub name: SmolStr,
    /// Pool label.
    pub label: String,
    /// Points available.
    pub total: i32,
    /// Points spent so far.
    pub spent: i32,
}

impl BudgetLedger {
    /// Points remaining (may be negative = overspent).
    #[must_use]
    pub fn remaining(&self) -> i32 {
        self.total - self.spent
    }

    /// Whether the pool is overspent.
    #[must_use]
    pub fn overspent(&self) -> bool {
        self.spent > self.total
    }

    /// Whether the pool has unspent points.
    #[must_use]
    pub fn underspent(&self) -> bool {
        self.spent < self.total
    }
}

/// A stage's spend plan: pools plus the cost rules that draw from them. The
/// framework computes ledgers and issues; the ruleset supplies the plan (pool
/// totals may be set from the player's priority choice at stage
/// instantiation).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetPlan {
    /// The pools.
    pub pools: Vec<PoolDef>,
    /// The cost rules.
    pub rules: Vec<CostRule>,
}

impl BudgetPlan {
    /// An empty plan.
    #[must_use]
    pub fn new() -> BudgetPlan {
        BudgetPlan {
            pools: Vec::new(),
            rules: Vec::new(),
        }
    }

    /// Builder-style: add a pool.
    #[must_use]
    pub fn pool(mut self, pool: PoolDef) -> BudgetPlan {
        self.pools.push(pool);
        self
    }

    /// Builder-style: add a cost rule.
    #[must_use]
    pub fn rule(mut self, rule: CostRule) -> BudgetPlan {
        self.rules.push(rule);
        self
    }

    /// Compute the ledgers: for each pool, the points spent by its rules
    /// against `doc`.
    #[must_use]
    pub fn assess(&self, schema: &SheetSchema, doc: &SheetDoc) -> Vec<BudgetLedger> {
        let mut ledgers: Vec<BudgetLedger> = self
            .pools
            .iter()
            .map(|p| BudgetLedger {
                name: p.name.clone(),
                label: p.label.clone(),
                total: p.total,
                spent: 0,
            })
            .collect();
        for rule in &self.rules {
            let Some(ledger) = ledgers.iter_mut().find(|l| l.name == rule.pool) else {
                continue; // rule references an undeclared pool; skip
            };
            for fid in rule.fields(schema) {
                let value = doc.number(fid).unwrap_or(i64::from(rule.base));
                let above = (value - i64::from(rule.base)).max(0);
                ledger.spent += i32::try_from(above).unwrap_or(i32::MAX) * rule.cost_per_dot;
            }
        }
        ledgers
    }

    /// The issues for this plan against `doc`: pool over/under-spend and
    /// per-field chargen-cap violations.
    #[must_use]
    pub fn issues(&self, schema: &SheetSchema, doc: &SheetDoc) -> Vec<ChargenIssue> {
        let mut issues = Vec::new();
        for l in self.assess(schema, doc) {
            if l.overspent() {
                issues.push(ChargenIssue::Overspent {
                    pool: l.name.clone(),
                    label: l.label.clone(),
                    by: l.spent - l.total,
                });
            } else if l.underspent() {
                issues.push(ChargenIssue::Underspent {
                    pool: l.name.clone(),
                    label: l.label.clone(),
                    by: l.total - l.spent,
                });
            }
        }
        for rule in &self.rules {
            if let Some(cap) = rule.cap {
                for fid in rule.fields(schema) {
                    if let Some(v) = doc.number(fid) {
                        let v = i32::try_from(v).unwrap_or(i32::MAX);
                        if v > cap {
                            issues.push(ChargenIssue::AboveCap {
                                field: fid.clone(),
                                cap,
                                got: v,
                            });
                        }
                    }
                }
            }
        }
        issues
    }
}

impl Default for BudgetPlan {
    fn default() -> Self {
        BudgetPlan::new()
    }
}

// ----------------------------------------------------------------------
// Stage machine
// ----------------------------------------------------------------------

/// A player's progress through a flow's stages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChargenProgress {
    /// The current stage index into [`ChargenFlow::stages`].
    pub stage: usize,
    /// Whether every stage has been passed.
    pub complete: bool,
}

impl ChargenProgress {
    /// Fresh progress at the first stage.
    #[must_use]
    pub fn start() -> ChargenProgress {
        ChargenProgress {
            stage: 0,
            complete: false,
        }
    }
}

impl Default for ChargenProgress {
    fn default() -> Self {
        ChargenProgress::start()
    }
}

/// Validate the current stage and, if clean, advance. On the last stage a
/// clean pass sets `complete`. Returns the blocking issues without advancing
/// if the stage is not yet consistent.
///
/// # Errors
///
/// Returns the current stage's [`ChargenIssue`]s if it does not yet validate.
pub fn try_advance(
    flow: &dyn ChargenFlow,
    sheet: &SheetDoc,
    progress: &mut ChargenProgress,
) -> Result<(), Vec<ChargenIssue>> {
    let stages = flow.stages();
    if progress.stage >= stages.len() {
        progress.complete = true;
        return Ok(());
    }
    let cur = &stages[progress.stage];
    let issues = flow.validate_stage(sheet, &cur.id);
    if !issues.is_empty() {
        return Err(issues);
    }
    if progress.stage + 1 >= stages.len() {
        progress.complete = true;
    } else {
        progress.stage += 1;
    }
    Ok(())
}

/// Finalize the flow (thin delegator to [`ChargenFlow::finalize`], for a
/// uniform driver call site).
///
/// # Errors
///
/// Propagates the flow's finalize error.
pub fn finalize(flow: &dyn ChargenFlow, sheet: &mut SheetDoc) -> Result<(), ChargenIssue> {
    flow.finalize(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FieldDef, SectionLayout, SheetSchemaBuilder, SheetSection};

    fn schema() -> SheetSchema {
        SheetSchemaBuilder::new("toy", 1)
            .section(
                SheetSection::new("attrs", "Attributes", SectionLayout::grid(1, 3))
                    .field(FieldDef::dots("strength", "Strength", 1, 5).category("physical"))
                    .field(FieldDef::dots("dexterity", "Dexterity", 1, 5).category("physical"))
                    .field(FieldDef::dots("stamina", "Stamina", 1, 5).category("physical")),
            )
            .unwrap()
            .build()
    }

    /// A plan: pool "physical" = 5 points; each physical attribute costs 1
    /// per dot above the free 1, capped at 4 during allocation.
    fn plan() -> BudgetPlan {
        BudgetPlan::new()
            .pool(PoolDef::new("physical", "Physical", 5))
            .rule(
                CostRule::new("physical", FieldSelector::Category("physical".into()), 1, 1)
                    .with_cap(4),
            )
    }

    #[test]
    fn assess_counts_dots_above_base() {
        let s = schema();
        let mut doc = SheetDoc::new_default(&s);
        // 1/1/1 → 0 spent.
        assert_eq!(plan().assess(&s, &doc)[0].spent, 0);
        // 3/2/2 → (2)+(1)+(1) = 4 spent.
        doc.set_number("strength", 3);
        doc.set_number("dexterity", 2);
        doc.set_number("stamina", 2);
        assert_eq!(plan().assess(&s, &doc)[0].spent, 4);
    }

    #[test]
    fn overspend_and_underspend_detected() {
        let s = schema();
        let mut doc = SheetDoc::new_default(&s);
        // Underspent: 4 spent of 5.
        doc.set_number("strength", 3);
        doc.set_number("dexterity", 2);
        doc.set_number("stamina", 2);
        let issues = plan().issues(&s, &doc);
        assert!(matches!(issues[0], ChargenIssue::Underspent { by: 1, .. }));

        // Overspent: 3+3+2 above base = 2+2+1 = ... set 4/4/2 → 3+3+1 = 7 of 5.
        doc.set_number("strength", 4);
        doc.set_number("dexterity", 4);
        doc.set_number("stamina", 2);
        let issues = plan().issues(&s, &doc);
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, ChargenIssue::Overspent { by: 2, .. }))
        );
    }

    #[test]
    fn cap_violation_detected() {
        let s = schema();
        let mut doc = SheetDoc::new_default(&s);
        doc.set_number("strength", 5); // over the cap of 4
        doc.set_number("dexterity", 1);
        doc.set_number("stamina", 1);
        let issues = plan().issues(&s, &doc);
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, ChargenIssue::AboveCap { cap: 4, got: 5, .. }))
        );
    }

    // A toy flow: one budgeted stage, finalize computes a derived total.
    struct ToyFlow {
        schema: SheetSchema,
    }

    impl ChargenFlow for ToyFlow {
        fn stages(&self) -> Vec<ChargenStage> {
            vec![
                ChargenStage::new("attrs", "Attributes", "Spend 5 physical points.")
                    .with_budget(plan()),
                ChargenStage::new("concept", "Concept", "Enter your concept."),
            ]
        }

        fn validate_stage(&self, sheet: &SheetDoc, stage: &SmolStr) -> Vec<ChargenIssue> {
            self.stages()
                .into_iter()
                .find(|s| &s.id == stage)
                .map(|s| s.assess(&self.schema, sheet))
                .unwrap_or_default()
        }

        fn finalize(&self, sheet: &mut SheetDoc) -> Result<(), ChargenIssue> {
            // Refuse if the budgeted stage is not exact.
            let issues = self.validate_stage(sheet, &SmolStr::new("attrs"));
            if !issues.is_empty() {
                return Err(issues.into_iter().next().unwrap());
            }
            // Derived: physical_total = str + dex + sta.
            let total = sheet.number("strength").unwrap_or(0)
                + sheet.number("dexterity").unwrap_or(0)
                + sheet.number("stamina").unwrap_or(0);
            sheet.set_number("physical_total", total);
            Ok(())
        }
    }

    #[test]
    fn state_machine_gates_and_finalizes() {
        let s = schema();
        let flow = ToyFlow { schema: s.clone() };
        let mut doc = SheetDoc::new_default(&s);
        let mut progress = ChargenProgress::start();

        // Stage 0 is underspent → advance refused, still on stage 0.
        assert!(try_advance(&flow, &doc, &mut progress).is_err());
        assert_eq!(progress.stage, 0);

        // Spend exactly 5: (3-1)+(3-1)+(2-1) = 2+2+1 = 5.
        doc.set_number("strength", 3);
        doc.set_number("dexterity", 3);
        doc.set_number("stamina", 2);
        assert!(try_advance(&flow, &doc, &mut progress).is_ok());
        assert_eq!(progress.stage, 1);
        assert!(!progress.complete);

        // Last stage (no budget) → clean, completes.
        assert!(try_advance(&flow, &doc, &mut progress).is_ok());
        assert!(progress.complete);

        // Finalize computes the derived total (3 + 3 + 2).
        finalize(&flow, &mut doc).unwrap();
        assert_eq!(doc.number("physical_total"), Some(8));
    }
}
