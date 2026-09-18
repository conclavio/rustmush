//! **The cost model for the LLM side of the design** (16b cut N.2, N3; doc 16 §6
//! "The cost run"; 16a Q9b).
//!
//! One table, read off `mush-llm`'s **existing** budget and audit records rather
//! than off a second set of counters: [`mush_llm::BudgetLedger`] already meters
//! every call at four scopes (global, feature, actor, NPC) and
//! [`mush_llm::AuditSink`] already folds per-`(feature, provider)` totals that
//! survive its ring's eviction. A squad's scope is its **commander's actor
//! scope** — [`mush_llm::CallCtx::with_actor`] on
//! [`crate::deliberation::request::deliberate`] is what puts it there — so
//! per-squad cost is a read and not a bookkeeping exercise.
//!
//! # The unit the table is in
//!
//! **Per unit-minute**, because that is the number the design can be costed with:
//! a match is a number of units for a number of minutes, and doc 16 §6's question
//! is what commanding them costs. A row's `*_per_unit_minute` columns are its own
//! figure over `units × minutes` of the run, so a squad row and the match row are
//! directly comparable and a bigger match is a multiplication.
//!
//! # The cap
//!
//! [`CostGuard`] is the hard stop 16a Q9b asks for: the run **stops before the
//! call that would pass the cap**, not after. It refuses on the worst call seen so
//! far rather than on an average, so a run with a cap of one dollar cannot spend
//! $1.40 because the last call was unusually long.

use serde::Serialize;

use mush_llm::{AuditSink, BudgetLedger};

/// The route every deliberation call is metered under — the feature's own
/// `route_key`, which is also the audit `feature` field and the per-feature budget
/// key.
pub const ROUTE: &str = "tacsim_deliberate";

/// One row of the table.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CostRow {
    /// `squad` or `match`.
    pub scope: String,
    /// The squad's rendered id (`red-1`), or the route for the match row.
    pub key: String,
    /// Calls the provider was asked for.
    pub calls: u64,
    /// Calls the model refused.
    pub refusals: u64,
    /// Calls that failed for any other reason (a transport error, a missing
    /// route, an open breaker).
    pub errors: u64,
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens consumed.
    pub output_tokens: u64,
    /// Input plus output.
    pub total_tokens: u64,
    /// Cost, USD.
    pub cost_usd: f64,
    /// Calls per unit-minute of the run.
    pub calls_per_unit_minute: f64,
    /// Tokens per unit-minute of the run.
    pub tokens_per_unit_minute: f64,
    /// USD per unit-minute of the run.
    pub usd_per_unit_minute: f64,
}

impl CostRow {
    /// The column headers, in row order — what the harness prints above the table
    /// and what the JSON's keys are.
    pub const COLUMNS: [&'static str; 12] = [
        "scope",
        "key",
        "calls",
        "refusals",
        "errors",
        "input_tokens",
        "output_tokens",
        "total_tokens",
        "cost_usd",
        "calls_per_unit_minute",
        "tokens_per_unit_minute",
        "usd_per_unit_minute",
    ];

    /// One line, fields separated by a space and named, in the shape every other
    /// `harness_*` line takes.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "scope={} key={} calls={} refusals={} errors={} input_tokens={} output_tokens={} \
             total_tokens={} cost_usd={:.6} calls_per_unit_minute={:.6} \
             tokens_per_unit_minute={:.3} usd_per_unit_minute={:.8}",
            self.scope,
            self.key,
            self.calls,
            self.refusals,
            self.errors,
            self.input_tokens,
            self.output_tokens,
            self.total_tokens,
            self.cost_usd,
            self.calls_per_unit_minute,
            self.tokens_per_unit_minute,
            self.usd_per_unit_minute
        )
    }

    fn rate(&mut self, unit_minutes: f64) {
        let per = if unit_minutes > 0.0 {
            unit_minutes
        } else {
            1.0
        };
        self.total_tokens = self.input_tokens.saturating_add(self.output_tokens);
        // `as f64` on a u64 count: the numbers here are calls and tokens in a run
        // of minutes, so the mantissa is nowhere near its limit, and a rate is a
        // float by definition. This is the one float in the cut and it never
        // reaches the engine (D-16-4 is about *arena state*).
        #[allow(clippy::cast_precision_loss)]
        {
            self.calls_per_unit_minute = self.calls as f64 / per;
            self.tokens_per_unit_minute = self.total_tokens as f64 / per;
        }
        self.usd_per_unit_minute = self.cost_usd / per;
    }
}

/// **The table the cost run prints.**
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CostTable {
    /// The route, so a reader knows which model this is a cost model of.
    pub route: String,
    /// The model id the route resolved to, as the audit rows report it.
    pub model: String,
    /// The provider id.
    pub provider: String,
    /// Minutes of harness play.
    pub minutes: f64,
    /// Units on the field.
    pub units: u32,
    /// Squads under command.
    pub squads: u32,
    /// `units × minutes`.
    pub unit_minutes: f64,
    /// The hard USD cap the run was given, if any.
    pub cap_usd: Option<f64>,
    /// Whether the run stopped because the next call would have passed the cap.
    pub stopped_at_cap: bool,
    /// One row per squad, then the match total.
    pub rows: Vec<CostRow>,
}

impl CostTable {
    /// The match row, if the table has one.
    #[must_use]
    pub fn total(&self) -> Option<&CostRow> {
        self.rows.iter().find(|row| row.scope == "match")
    }
}

/// **Build the table** from the two records `mush-llm` already keeps.
///
/// `squads` is `(rendered squad id, commander object id)`; each becomes one row
/// read from the ledger's **actor** scope, and the match row is read from its
/// **feature** scope. Calls, refusals and errors come from the audit sink: its
/// per-`(feature, provider)` totals for the match, and its ring for the per-squad
/// split.
///
/// The ring is bounded (`AuditSink::new`'s capacity), so a run long enough to
/// evict rows under-reports a squad's *call count* while its tokens and cost stay
/// exact — the ledger never evicts. A five-minute run at
/// `[tacsim.arena].deliberate_min_ms` cannot reach the default capacity, and the
/// harness says so on the line it prints.
#[must_use]
pub fn cost_table(
    ledger: &BudgetLedger,
    audit: &AuditSink,
    squads: &[(String, mush_core::ObjId)],
    units: u32,
    minutes: f64,
    cap_usd: Option<f64>,
    stopped_at_cap: bool,
) -> CostTable {
    let unit_minutes = f64::from(units) * minutes;
    let totals = audit
        .totals()
        .into_iter()
        .find(|row| row.feature == ROUTE)
        .unwrap_or_default();
    let recent = audit.recent(usize::MAX);
    let mut rows: Vec<CostRow> = Vec::with_capacity(squads.len() + 1);
    for (key, commander) in squads {
        let counters = ledger.actor(*commander);
        let mine: Vec<&mush_llm::AuditRecord> = recent
            .iter()
            .filter(|record| record.feature == ROUTE && record.actor == Some(*commander))
            .collect();
        let mut row = CostRow {
            scope: "squad".to_string(),
            key: key.clone(),
            calls: mine.len() as u64,
            refusals: mine
                .iter()
                .filter(|record| record.outcome == mush_llm::Outcome::Refused)
                .count() as u64,
            errors: mine
                .iter()
                .filter(|record| {
                    matches!(
                        record.outcome,
                        mush_llm::Outcome::Error
                            | mush_llm::Outcome::Budget
                            | mush_llm::Outcome::Disabled
                    )
                })
                .count() as u64,
            input_tokens: counters.input_tokens,
            output_tokens: counters.output_tokens,
            total_tokens: 0,
            cost_usd: counters.cost_usd,
            ..CostRow::default()
        };
        row.rate(unit_minutes);
        rows.push(row);
    }
    let feature = ledger.feature(ROUTE);
    let mut total = CostRow {
        scope: "match".to_string(),
        key: ROUTE.to_string(),
        calls: totals.calls,
        refusals: totals.refusals,
        errors: totals
            .errors
            .saturating_add(totals.budget_trips)
            .saturating_add(totals.disabled),
        input_tokens: feature.input_tokens,
        output_tokens: feature.output_tokens,
        total_tokens: 0,
        cost_usd: feature.cost_usd,
        ..CostRow::default()
    };
    total.rate(unit_minutes);
    rows.push(total);
    CostTable {
        route: ROUTE.to_string(),
        model: recent
            .iter()
            .rev()
            .find(|record| record.feature == ROUTE)
            .map_or_else(|| "-".to_string(), |record| record.model.clone()),
        provider: totals.provider,
        minutes,
        units,
        squads: u32::try_from(squads.len()).unwrap_or(u32::MAX),
        unit_minutes,
        cap_usd,
        stopped_at_cap,
        rows,
    }
}

/// **The hard USD cap** (16a Q9b: *a hard USD cap per run*).
///
/// The run asks before every call. It refuses on the **worst call seen so far**
/// rather than on the mean, so the cap is a ceiling on what the run can spend and
/// not on what it has spent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CostGuard {
    cap_usd: Option<f64>,
    worst_call_usd: f64,
}

impl CostGuard {
    /// A guard with this cap; `None` is no cap at all, which is what a run against
    /// the mock provider uses.
    #[must_use]
    pub const fn new(cap_usd: Option<f64>) -> CostGuard {
        CostGuard {
            cap_usd,
            worst_call_usd: 0.0,
        }
    }

    /// Whether one more call may be made, given what the feature has spent.
    ///
    /// The first call is always allowed: a cap that refused before any call had a
    /// cost to compare against would make every run report nothing.
    #[must_use]
    pub fn may_call(&self, spent_usd: f64) -> bool {
        match self.cap_usd {
            None => true,
            Some(cap) => spent_usd + self.worst_call_usd <= cap,
        }
    }

    /// Note what a call cost, so the next refusal is made against the worst case.
    pub fn observe(&mut self, spent_before_usd: f64, spent_after_usd: f64) {
        let one = spent_after_usd - spent_before_usd;
        if one > self.worst_call_usd {
            self.worst_call_usd = one;
        }
    }

    /// The cap, for the report.
    #[must_use]
    pub const fn cap_usd(&self) -> Option<f64> {
        self.cap_usd
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::ObjId;
    use mush_llm::{AuditRecord, Outcome, Usage};

    fn record(actor: ObjId, outcome: Outcome, input: u64, output: u64, cost: f64) -> AuditRecord {
        AuditRecord {
            ts: 1,
            feature: ROUTE.to_string(),
            provider: "openrouter".to_string(),
            model: "z-ai/glm-5.3-flash".to_string(),
            actor: Some(actor),
            npc: None,
            prompt_sha256: String::new(),
            prompt_summary: String::new(),
            response_summary: String::new(),
            input_tokens: input,
            output_tokens: output,
            cost_usd: Some(cost),
            latency_ms: 10,
            outcome,
        }
    }

    #[test]
    fn the_table_has_a_row_per_squad_and_a_match_total() {
        let ledger = BudgetLedger::new();
        let audit = AuditSink::new(64);
        let limits = |actor: ObjId| mush_llm::ScopeLimits {
            global: Some(mush_llm::BudgetLimit::default()),
            feature: Some((ROUTE.to_string(), mush_llm::BudgetLimit::default())),
            actor: Some((actor, mush_llm::BudgetLimit::default())),
            npc: None,
        };
        let usage = |input: u64, output: u64, cost: f64| Usage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
            reasoning_tokens: 0,
            cost_usd: Some(cost),
        };
        ledger.settle(&limits(ObjId(4)), &usage(100, 20, 0.001));
        ledger.settle(&limits(ObjId(9)), &usage(200, 40, 0.002));
        audit.record(record(ObjId(4), Outcome::Ok, 100, 20, 0.001));
        audit.record(record(ObjId(9), Outcome::Ok, 200, 40, 0.002));
        audit.record(record(ObjId(9), Outcome::Refused, 0, 0, 0.0));

        let table = cost_table(
            &ledger,
            &audit,
            &[
                ("red-1".to_string(), ObjId(4)),
                ("blue-1".to_string(), ObjId(9)),
            ],
            24,
            5.0,
            Some(2.0),
            false,
        );
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.unit_minutes, 120.0);
        assert_eq!(table.squads, 2);
        assert_eq!(table.model, "z-ai/glm-5.3-flash");
        let red = &table.rows[0];
        assert_eq!(red.key, "red-1");
        assert_eq!(red.calls, 1);
        assert_eq!(red.total_tokens, 120);
        assert!((red.usd_per_unit_minute - 0.001 / 120.0).abs() < 1e-12);
        let blue = &table.rows[1];
        assert_eq!(blue.calls, 2);
        assert_eq!(blue.refusals, 1);
        let total = table.total().expect("a match row");
        assert_eq!(total.scope, "match");
        assert_eq!(total.calls, 3);
        assert_eq!(total.total_tokens, 360);
        assert!((total.cost_usd - 0.003).abs() < 1e-12);
        assert!(total.line().contains("scope=match"));
        assert_eq!(CostRow::COLUMNS.len(), 12);
    }

    #[test]
    fn the_guard_stops_before_the_call_that_would_pass_the_cap() {
        let mut guard = CostGuard::new(Some(0.10));
        assert!(guard.may_call(0.0), "the first call is always allowed");
        guard.observe(0.0, 0.04);
        assert!(guard.may_call(0.04), "0.04 + 0.04 is inside 0.10");
        guard.observe(0.04, 0.08);
        assert!(!guard.may_call(0.08), "0.08 + 0.04 would pass 0.10");
        assert_eq!(guard.cap_usd(), Some(0.10));
        let free = CostGuard::new(None);
        assert!(free.may_call(1_000.0));
    }
}
