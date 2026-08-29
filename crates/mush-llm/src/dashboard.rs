//! Safe aggregate accessors for the LLM cost dashboard (M10c, design 03 §9
//! phase 5's "cost dashboards"): today's budget-vs-cap by scope, the top
//! spenders, a best-effort last-week series by feature/provider, and a
//! recent-calls tail. Everything here is assembled from
//! [`BudgetLedger`]/[`AuditSink`]'s own accessors — nothing new is stored,
//! nothing here mutates.
//!
//! **Aggregates only.** [`RecentCall`] deliberately does NOT carry
//! [`crate::audit::AuditRecord`]'s `prompt_sha256`/`prompt_summary`/
//! `response_summary` fields — those exist so staff can correlate a row
//! with a specific in-game exchange from the terminal `@llm/log`, but they
//! hold real prompt/response text (player speech, generated room prose) and
//! must never reach a dashboard payload. Every type here is either a
//! counter, a cap, a timestamp, a model/feature/provider name, or an
//! [`ObjId`] the audit trail already carries — nothing else.

use std::collections::{BTreeSet, HashMap};

use mush_core::ObjId;

use crate::audit::{AuditSink, DaySlice};
use crate::budget::{BudgetLedger, BudgetLimit, Counters};
use crate::config::BudgetConfig;

/// How many trailing UTC days [`build`] covers.
const SERIES_DAYS: u64 = 7;
/// How many top spenders [`build`] keeps per top-actor/top-NPC list.
const TOP_N: usize = 10;
/// How many recent-calls rows [`build`] keeps.
const RECENT_N: usize = 20;

/// One scope's today-vs-cap snapshot: `"global"`, `"feature:<route>"`,
/// `"actor_default"`, or `"npc_default"`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BudgetStatus {
    /// The scope's label (see the type doc for the fixed set of values).
    pub scope: String,
    /// Input tokens consumed today (UTC).
    pub input_tokens: u64,
    /// Output tokens consumed today (UTC).
    pub output_tokens: u64,
    /// Cost consumed today (UTC), USD.
    pub cost_usd: f64,
    /// The configured token ceiling, if any.
    pub tokens_cap: Option<u64>,
    /// The configured cost ceiling, if any.
    pub cost_cap: Option<f64>,
    /// `tokens_cap` minus tokens used, when a token cap is configured.
    pub tokens_remaining: Option<u64>,
    /// `cost_cap` minus cost used (never negative), when a cost cap is
    /// configured. The shipped example's most important caps are cost-only
    /// (`per_actor_default`, `worldgen_room`), so a cost-only scope needs a
    /// remaining figure to drive its meter (M10c) — without it those scopes
    /// rendered as bare numbers with no approach-the-cap signal.
    pub cost_remaining: Option<f64>,
}

impl BudgetStatus {
    fn new(scope: impl Into<String>, used: Counters, cap: Option<BudgetLimit>) -> BudgetStatus {
        let tokens_cap = cap.and_then(|c| c.tokens);
        let cost_cap = cap.and_then(|c| c.cost_usd);
        BudgetStatus {
            scope: scope.into(),
            input_tokens: used.input_tokens,
            output_tokens: used.output_tokens,
            cost_usd: used.cost_usd,
            tokens_cap,
            cost_cap,
            tokens_remaining: tokens_cap.map(|c| c.saturating_sub(used.total_tokens())),
            cost_remaining: cost_cap.map(|c| (c - used.cost_usd).max(0.0)),
        }
    }
}

/// One actor or NPC's spend today (UTC) — a "top spenders" row. Carries only
/// the [`ObjId`]; the caller resolves a display name against `&World` (this
/// crate holds no world reference — treaty ruling 3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpendRow {
    /// The builder or NPC.
    pub obj: ObjId,
    /// Input tokens consumed today.
    pub input_tokens: u64,
    /// Output tokens consumed today.
    pub output_tokens: u64,
    /// Cost consumed today, USD.
    pub cost_usd: f64,
}

/// One recent call — audit **summary** fields only (see the module doc for
/// why the prompt/response fields are excluded on purpose).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecentCall {
    /// Wall-clock unix seconds.
    pub ts: u64,
    /// Feature route name.
    pub feature: String,
    /// Provider id.
    pub provider: String,
    /// Model id (as routed).
    pub model: String,
    /// Triggering actor (builder/player), if any.
    pub actor: Option<ObjId>,
    /// NPC, if this was an agent turn.
    pub npc: Option<ObjId>,
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Cost, USD, if known.
    pub cost_usd: Option<f64>,
    /// End-to-end latency, ms.
    pub latency_ms: u64,
    /// The wire outcome label (`"ok"`, `"refused"`, `"error"`, `"budget"`,
    /// `"disabled"`).
    pub outcome: &'static str,
}

/// The full dashboard aggregate — everything the `llm.dashboard` query
/// handler and the `@llm/dashboard` terminal command render. Build with
/// [`build`] (or [`crate::LlmService::dashboard`], which supplies the
/// service's own state).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DashboardSnapshot {
    /// Whether the global LLM kill switch is currently engaged.
    pub kill_switch_engaged: bool,
    /// `"global"` first, then one row per feature that has spent anything
    /// today or carries a configured cap, then `"actor_default"` /
    /// `"npc_default"` when configured.
    pub budgets: Vec<BudgetStatus>,
    /// Top builders by spend today, descending.
    pub top_actors: Vec<SpendRow>,
    /// Top NPCs by spend today, descending.
    pub top_npcs: Vec<SpendRow>,
    /// Best-effort last-week series, oldest day first — see [`DaySlice`].
    pub daily_series: Vec<DaySlice>,
    /// The most recent calls, newest first.
    pub recent_calls: Vec<RecentCall>,
}

/// Build the dashboard snapshot. `known_features` is every route name the
/// service has configured (so a feature with a cap but zero spend today
/// still gets a row); `kill_switch_engaged` is the global switch's current
/// state. [`crate::LlmService::dashboard`] is the convenience entry point
/// that supplies both from a live service.
#[must_use]
pub fn build(
    ledger: &BudgetLedger,
    audit: &AuditSink,
    budgets: &BudgetConfig,
    known_features: impl IntoIterator<Item = String>,
    kill_switch_engaged: bool,
) -> DashboardSnapshot {
    let used: HashMap<String, Counters> = ledger.feature_totals().into_iter().collect();
    let mut features: BTreeSet<String> = known_features.into_iter().collect();
    features.extend(used.keys().cloned());
    features.extend(budgets.per_feature.keys().cloned());

    let mut rows = vec![BudgetStatus::new("global", ledger.global(), budgets.global)];
    for f in features {
        let u = used.get(&f).copied().unwrap_or_default();
        let cap = budgets.per_feature.get(&f).copied();
        if u.total_tokens() == 0 && u.cost_usd == 0.0 && cap.is_none() {
            continue; // a route the service knows about but never used today
        }
        rows.push(BudgetStatus::new(format!("feature:{f}"), u, cap));
    }
    if let Some(cap) = budgets.per_actor_default {
        rows.push(BudgetStatus::new(
            "actor_default",
            Counters::default(),
            Some(cap),
        ));
    }
    if let Some(cap) = budgets.per_npc_default {
        rows.push(BudgetStatus::new(
            "npc_default",
            Counters::default(),
            Some(cap),
        ));
    }

    let spend_row = |(obj, c): (ObjId, Counters)| SpendRow {
        obj,
        input_tokens: c.input_tokens,
        output_tokens: c.output_tokens,
        cost_usd: c.cost_usd,
    };
    let top_actors = ledger
        .top_actors(TOP_N)
        .into_iter()
        .map(spend_row)
        .collect();
    let top_npcs = ledger.top_npcs(TOP_N).into_iter().map(spend_row).collect();

    let daily_series = audit.daily_series(SERIES_DAYS);

    let mut recent_calls: Vec<RecentCall> = audit
        .recent(RECENT_N)
        .into_iter()
        .map(|r| RecentCall {
            ts: r.ts,
            feature: r.feature,
            provider: r.provider,
            model: r.model,
            actor: r.actor,
            npc: r.npc,
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            cost_usd: r.cost_usd,
            latency_ms: r.latency_ms,
            outcome: r.outcome.as_str(),
        })
        .collect();
    recent_calls.reverse(); // AuditSink::recent is oldest-first; dashboard wants newest-first

    DashboardSnapshot {
        kill_switch_engaged,
        budgets: rows,
        top_actors,
        top_npcs,
        daily_series,
        recent_calls,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditRecord, Outcome};
    use crate::budget::ScopeLimits;
    use crate::types::Usage;

    fn usage(input: u64, output: u64, cost: Option<f64>) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
            reasoning_tokens: 0,
            cost_usd: cost,
        }
    }

    #[test]
    fn build_combines_budgets_top_spenders_and_recent_calls() {
        let ledger = BudgetLedger::new();
        let audit = AuditSink::new(64);
        let alice = ObjId(5);
        let npc = ObjId(6);

        ledger.settle(
            &ScopeLimits {
                global: Some(BudgetLimit::tokens(10_000)),
                feature: Some(("worldgen_room".to_string(), BudgetLimit::tokens(1_000))),
                actor: Some((alice, BudgetLimit::tokens(1_000))),
                ..Default::default()
            },
            &usage(100, 50, Some(0.05)),
        );
        ledger.settle(
            &ScopeLimits {
                npc: Some((npc, BudgetLimit::tokens(500))),
                ..Default::default()
            },
            &usage(10, 10, Some(0.01)),
        );
        audit.record(AuditRecord {
            ts: now_unix_for_test(),
            feature: "worldgen_room".into(),
            provider: "openrouter".into(),
            model: "z-ai/glm-5.3".into(),
            actor: Some(alice),
            npc: None,
            prompt_sha256: "deadbeef".into(),
            prompt_summary: "a secret room prompt".into(),
            response_summary: "a secret generated desc".into(),
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: Some(0.05),
            latency_ms: 1200,
            outcome: Outcome::Ok,
        });

        let budgets = BudgetConfig {
            global: Some(BudgetLimit::tokens(10_000)),
            per_feature: [("worldgen_room".to_string(), BudgetLimit::tokens(1_000))]
                .into_iter()
                .collect(),
            per_actor_default: Some(BudgetLimit::tokens(1_000)),
            per_npc_default: Some(BudgetLimit::tokens(500)),
        };
        let snap = build(
            &ledger,
            &audit,
            &budgets,
            [
                "worldgen_room".to_string(),
                "npc_dialogue_major".to_string(),
            ],
            false,
        );

        assert!(!snap.kill_switch_engaged);

        // Only the first `settle()` named a `global` scope (the second was
        // NPC-only), so global holds just that call's usage.
        let global = snap.budgets.iter().find(|b| b.scope == "global").unwrap();
        assert_eq!(global.input_tokens, 100);
        assert_eq!(global.output_tokens, 50);

        let wg = snap
            .budgets
            .iter()
            .find(|b| b.scope == "feature:worldgen_room")
            .expect("worldgen_room budget row");
        assert_eq!(wg.tokens_cap, Some(1_000));
        assert_eq!(wg.tokens_remaining, Some(1_000 - 150));

        // npc_dialogue_major never spent and has no configured cap: no row.
        assert!(
            !snap
                .budgets
                .iter()
                .any(|b| b.scope == "feature:npc_dialogue_major")
        );

        assert_eq!(snap.top_actors.len(), 1);
        assert_eq!(snap.top_actors[0].obj, alice);
        assert_eq!(snap.top_npcs.len(), 1);
        assert_eq!(snap.top_npcs[0].obj, npc);

        assert_eq!(snap.recent_calls.len(), 1);
        let call = &snap.recent_calls[0];
        assert_eq!(call.feature, "worldgen_room");
        assert_eq!(call.actor, Some(alice));
        assert_eq!(call.outcome, "ok");
        // NO prompt/response text anywhere on the dashboard type.
        let debug = format!("{call:?}");
        assert!(!debug.contains("secret"));

        assert!(!snap.daily_series.is_empty());
    }

    fn now_unix_for_test() -> u64 {
        crate::audit::now_unix()
    }
}
