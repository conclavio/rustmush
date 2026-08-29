//! A lightweight in-process metrics registry (PLAN "Observability").
//!
//! Hand-rolled atomics — no `metrics` crate, no exporter dependency, no
//! network. One `Arc<Metrics>` is shared by everything that produces numbers
//! (the transports bump connection counters as sockets open and close; the
//! `mushd` world task publishes the world-owned gauges — object counts, loop
//! lag, store health, scheduler depth, NPC and LLM aggregates — each sample)
//! and read back by the two HTTP endpoints this crate serves:
//!
//! - `GET /healthz` — a tiny liveness JSON (`ok`, `uptime_s`, `version`,
//!   `connections`, `loop_lag_ms`, `world_loop_unixtime`, `store_ok`), read
//!   lock-free straight from the atomics.
//! - `GET /metrics` — Prometheus text exposition of every counter/gauge/
//!   histogram, gated behind `web.metrics_enabled` (opt-in, default off).
//!
//! ## What is safe to expose
//!
//! `/metrics` binds to the same listener as the web client, so it may be
//! public. Everything here is a **non-sensitive aggregate**: counts, gauges,
//! and cost totals. No metric is ever keyed by player, account, IP, token, or
//! prompt text — the LLM rows are keyed only by feature route + provider, and
//! the audit layer that feeds them already scrubs prompts and keys. Keep it
//! that way: never add a label carrying per-player or secret data.
//!
//! ## Freshness
//!
//! Connection counters are event-driven, so they are always live. The
//! world-owned gauges are refreshed by the world loop's periodic sample, so
//! they can lag reality by up to the sample interval when the server is
//! quiet — acceptable for observability (a quiet server is one where those
//! numbers are not changing).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

/// Loop-lag histogram bucket upper bounds, milliseconds (Prometheus `le`).
/// The final `+Inf` bucket is implicit (`total` minus the sum of these).
pub const LAG_BUCKETS_MS: [u64; 12] = [1, 2, 5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000];

/// Cumulative LLM counters for one (feature route, provider) pair, since boot.
/// Sourced from the `mush-llm` audit aggregate; every field is monotonic, so
/// they render as Prometheus counters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LlmFeatureStat {
    /// Feature route name (e.g. `worldgen_room`, `npc_dialogue_major`).
    pub feature: String,
    /// Provider id (e.g. `openrouter`, `anthropic`).
    pub provider: String,
    /// Total calls attempted (all outcomes).
    pub calls: u64,
    /// Calls the model refused.
    pub refusals: u64,
    /// Calls refused pre-flight by a budget ceiling.
    pub budget_trips: u64,
    /// Calls refused pre-flight by a kill switch.
    pub disabled: u64,
    /// Provider/transport errors.
    pub errors: u64,
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens consumed.
    pub output_tokens: u64,
    /// Cost consumed, USD.
    pub cost_usd: f64,
}

/// Today's global LLM spend (a gauge — resets at the UTC day rollover), from
/// the budget ledger. Distinct from the cumulative [`LlmFeatureStat`] counters.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LlmTodaySpend {
    /// Input tokens today.
    pub input_tokens: u64,
    /// Output tokens today.
    pub output_tokens: u64,
    /// Cost today, USD.
    pub cost_usd: f64,
}

/// The shared metrics registry. Cheap to clone via `Arc`; every field is an
/// atomic (or a small mutex-guarded vector for the dynamic LLM rows).
#[derive(Debug, Default)]
pub struct Metrics {
    // -- connections (event-driven, always live) --
    ssh_open_total: AtomicU64,
    ssh_current: AtomicI64,
    web_open_total: AtomicU64,
    web_current: AtomicI64,

    // -- sessions / commands (world-task published / event-driven) --
    sessions_current: AtomicI64,
    connections_current: AtomicI64,
    commands_total: AtomicU64,

    // -- world loop (published each sample) --
    loop_lag_last_ms: AtomicU64,
    loop_lag_max_ms: AtomicU64,
    lag_buckets: [AtomicU64; 12],
    lag_count: AtomicU64,
    lag_sum_ms: AtomicU64,
    /// Unix time (seconds) at the start of the world loop's most recent
    /// iteration — a liveness heartbeat set **every** iteration (not the
    /// throttled sample). A scraper alerts on staleness: a loop wedged inside a
    /// sync handler stops advancing this while `loop_lag_last_ms` freezes at
    /// its last low value, so this is what distinguishes "wedged" from "fast".
    /// (A completely idle loop with no due timer also stops advancing it, so
    /// combine with command/connection activity when reading it.)
    world_loop_last_iter_unix: AtomicI64,
    scheduler_queue_depth: AtomicI64,

    // -- object counts by kind (published each sample) --
    obj_rooms: AtomicI64,
    obj_exits: AtomicI64,
    obj_things: AtomicI64,
    obj_players: AtomicI64,
    obj_npcs: AtomicI64,

    // -- store health (published each sample; ack-time set by the writer) --
    store_read_only: AtomicBool,
    store_write_queue_depth: AtomicI64,
    store_last_ack_unix: AtomicI64,

    // -- NPC agents (published each sample) --
    npc_active: AtomicI64,
    npc_thinking: AtomicI64,

    // -- LLM (published each sample) --
    llm: Mutex<LlmMetrics>,
}

#[derive(Debug, Default)]
struct LlmMetrics {
    /// Whether an LLM service is configured/enabled at all.
    enabled: bool,
    /// Whether the global kill switch is engaged.
    globally_disabled: bool,
    /// Cumulative per-(feature, provider) counters.
    features: Vec<LlmFeatureStat>,
    /// Today's global spend gauge.
    today: LlmTodaySpend,
}

impl Metrics {
    /// A fresh, all-zero registry.
    #[must_use]
    pub fn new() -> Metrics {
        Metrics::default()
    }

    // -- connection counters (called by the transports) -------------------

    /// One SSH transport connection accepted.
    pub fn ssh_connection_opened(&self) {
        self.ssh_open_total.fetch_add(1, Ordering::Relaxed);
        self.ssh_current.fetch_add(1, Ordering::Relaxed);
    }

    /// One SSH transport connection closed.
    pub fn ssh_connection_closed(&self) {
        self.ssh_current.fetch_sub(1, Ordering::Relaxed);
    }

    /// One web (WebSocket) transport connection accepted.
    pub fn web_connection_opened(&self) {
        self.web_open_total.fetch_add(1, Ordering::Relaxed);
        self.web_current.fetch_add(1, Ordering::Relaxed);
    }

    /// One web transport connection closed.
    pub fn web_connection_closed(&self) {
        self.web_current.fetch_sub(1, Ordering::Relaxed);
    }

    /// Count one processed command (world task, per `Inbound::Cmd`).
    pub fn command_processed(&self) {
        self.commands_total.fetch_add(1, Ordering::Relaxed);
    }

    // -- world-loop sampling ----------------------------------------------

    /// Record one loop-iteration lag observation (feeds last/max + histogram).
    /// Called every iteration by the watchdog — cheap (a handful of atomics).
    pub fn observe_loop_lag(&self, ms: u64) {
        self.loop_lag_last_ms.store(ms, Ordering::Relaxed);
        self.loop_lag_max_ms.fetch_max(ms, Ordering::Relaxed);
        self.lag_count.fetch_add(1, Ordering::Relaxed);
        self.lag_sum_ms.fetch_add(ms, Ordering::Relaxed);
        for (i, &bound) in LAG_BUCKETS_MS.iter().enumerate() {
            if ms <= bound {
                self.lag_buckets[i].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Record that the world loop began an iteration at `unix_s` — the
    /// liveness heartbeat. Called every iteration (cheap: one atomic store),
    /// unlike the throttled [`publish_world`](Self::publish_world) sample.
    pub fn set_world_loop_iteration(&self, unix_s: i64) {
        self.world_loop_last_iter_unix
            .store(unix_s, Ordering::Relaxed);
    }

    /// The world loop's most recent iteration timestamp (Unix seconds; 0 if it
    /// has never run).
    #[must_use]
    pub fn world_loop_last_iter_unix(&self) -> i64 {
        self.world_loop_last_iter_unix.load(Ordering::Relaxed)
    }

    /// The `/healthz` liveness JSON, read **lock-free** straight from the
    /// atomics it needs — no [`snapshot`](Self::snapshot) (which would lock the
    /// LLM mutex and clone the feature vec) for four scalars. `/healthz` is
    /// unauthenticated and unthrottled, so keep it allocation- and
    /// contention-light.
    #[must_use]
    pub fn health_json(&self, uptime_s: u64, version: &str) -> serde_json::Value {
        serde_json::json!({
            "ok": true,
            "uptime_s": uptime_s,
            "version": version,
            "connections": self.ssh_current.load(Ordering::Relaxed)
                + self.web_current.load(Ordering::Relaxed),
            "loop_lag_ms": self.loop_lag_last_ms.load(Ordering::Relaxed),
            "world_loop_unixtime": self.world_loop_last_iter_unix.load(Ordering::Relaxed),
            "store_ok": !self.store_read_only.load(Ordering::Relaxed),
        })
    }

    /// Publish the world-owned gauges from one world-loop sample.
    pub fn publish_world(&self, w: WorldSample) {
        self.sessions_current
            .store(w.sessions_current, Ordering::Relaxed);
        self.connections_current
            .store(w.connections_current, Ordering::Relaxed);
        self.scheduler_queue_depth
            .store(w.scheduler_queue_depth, Ordering::Relaxed);
        self.obj_rooms.store(w.rooms, Ordering::Relaxed);
        self.obj_exits.store(w.exits, Ordering::Relaxed);
        self.obj_things.store(w.things, Ordering::Relaxed);
        self.obj_players.store(w.players, Ordering::Relaxed);
        self.obj_npcs.store(w.npcs, Ordering::Relaxed);
        self.store_read_only
            .store(w.store_read_only, Ordering::Relaxed);
        self.npc_active.store(w.npc_active, Ordering::Relaxed);
        self.npc_thinking.store(w.npc_thinking, Ordering::Relaxed);
    }

    /// Publish the store write-queue depth (world task on each flush) and the
    /// last-ack timestamp (the writer task on each acked write).
    pub fn set_store_write_queue_depth(&self, depth: i64) {
        self.store_write_queue_depth.store(depth, Ordering::Relaxed);
    }

    /// Bump the store write-queue depth by `delta` (writer enqueue/ack).
    pub fn add_store_write_queue_depth(&self, delta: i64) {
        self.store_write_queue_depth
            .fetch_add(delta, Ordering::Relaxed);
    }

    /// Record that the store acknowledged a write at `unix_s`.
    pub fn set_store_last_ack_unix(&self, unix_s: i64) {
        self.store_last_ack_unix.store(unix_s, Ordering::Relaxed);
    }

    /// Replace the LLM aggregate (world task on each sample).
    pub fn publish_llm(
        &self,
        enabled: bool,
        globally_disabled: bool,
        features: Vec<LlmFeatureStat>,
        today: LlmTodaySpend,
    ) {
        let mut g = self.llm.lock().expect("metrics llm lock");
        g.enabled = enabled;
        g.globally_disabled = globally_disabled;
        g.features = features;
        g.today = today;
    }

    // -- reads -------------------------------------------------------------

    /// Current total live connections across both transports (SSH + web).
    #[must_use]
    pub fn connections_current(&self) -> i64 {
        self.ssh_current.load(Ordering::Relaxed) + self.web_current.load(Ordering::Relaxed)
    }

    /// The most recent loop-lag observation, milliseconds.
    #[must_use]
    pub fn loop_lag_last_ms(&self) -> u64 {
        self.loop_lag_last_ms.load(Ordering::Relaxed)
    }

    /// Whether the store is currently healthy (not in read-only mode).
    #[must_use]
    pub fn store_ok(&self) -> bool {
        !self.store_read_only.load(Ordering::Relaxed)
    }

    /// A full point-in-time copy of every metric, for rendering.
    #[must_use]
    pub fn snapshot(&self) -> MetricsSnapshot {
        let g = self.llm.lock().expect("metrics llm lock");
        MetricsSnapshot {
            ssh_open_total: self.ssh_open_total.load(Ordering::Relaxed),
            ssh_current: self.ssh_current.load(Ordering::Relaxed),
            web_open_total: self.web_open_total.load(Ordering::Relaxed),
            web_current: self.web_current.load(Ordering::Relaxed),
            sessions_current: self.sessions_current.load(Ordering::Relaxed),
            connections_current: self.connections_current.load(Ordering::Relaxed),
            commands_total: self.commands_total.load(Ordering::Relaxed),
            loop_lag_last_ms: self.loop_lag_last_ms.load(Ordering::Relaxed),
            loop_lag_max_ms: self.loop_lag_max_ms.load(Ordering::Relaxed),
            lag_buckets: std::array::from_fn(|i| self.lag_buckets[i].load(Ordering::Relaxed)),
            lag_count: self.lag_count.load(Ordering::Relaxed),
            lag_sum_ms: self.lag_sum_ms.load(Ordering::Relaxed),
            world_loop_last_iter_unix: self.world_loop_last_iter_unix.load(Ordering::Relaxed),
            scheduler_queue_depth: self.scheduler_queue_depth.load(Ordering::Relaxed),
            rooms: self.obj_rooms.load(Ordering::Relaxed),
            exits: self.obj_exits.load(Ordering::Relaxed),
            things: self.obj_things.load(Ordering::Relaxed),
            players: self.obj_players.load(Ordering::Relaxed),
            npcs: self.obj_npcs.load(Ordering::Relaxed),
            store_read_only: self.store_read_only.load(Ordering::Relaxed),
            store_write_queue_depth: self.store_write_queue_depth.load(Ordering::Relaxed),
            store_last_ack_unix: self.store_last_ack_unix.load(Ordering::Relaxed),
            npc_active: self.npc_active.load(Ordering::Relaxed),
            npc_thinking: self.npc_thinking.load(Ordering::Relaxed),
            llm_enabled: g.enabled,
            llm_globally_disabled: g.globally_disabled,
            llm_features: g.features.clone(),
            llm_today: g.today,
        }
    }
}

/// The world-owned gauges published each world-loop sample.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorldSample {
    /// Live sessions.
    pub sessions_current: i64,
    /// Live connections (session-manager count; all transports).
    pub connections_current: i64,
    /// Scheduler entries pending.
    pub scheduler_queue_depth: i64,
    /// Room objects (not pending destruction).
    pub rooms: i64,
    /// Exit objects.
    pub exits: i64,
    /// Thing objects.
    pub things: i64,
    /// Player objects.
    pub players: i64,
    /// NPC objects.
    pub npcs: i64,
    /// Whether the store is in read-only mode.
    pub store_read_only: bool,
    /// NPCs with a live runtime (stimulated at least once).
    pub npc_active: i64,
    /// NPCs with an in-flight LLM turn.
    pub npc_thinking: i64,
}

/// A rendered-ready copy of every metric.
#[derive(Clone, Debug)]
pub struct MetricsSnapshot {
    /// SSH connections accepted since boot.
    pub ssh_open_total: u64,
    /// SSH connections live now.
    pub ssh_current: i64,
    /// Web connections accepted since boot.
    pub web_open_total: u64,
    /// Web connections live now.
    pub web_current: i64,
    /// Live sessions.
    pub sessions_current: i64,
    /// Live connections (all transports, session-manager count).
    pub connections_current: i64,
    /// Commands processed since boot.
    pub commands_total: u64,
    /// Most recent loop lag, ms.
    pub loop_lag_last_ms: u64,
    /// Worst loop lag since boot, ms.
    pub loop_lag_max_ms: u64,
    /// Cumulative loop-lag histogram bucket counts (aligned to
    /// [`LAG_BUCKETS_MS`]).
    pub lag_buckets: [u64; 12],
    /// Total loop-lag observations.
    pub lag_count: u64,
    /// Sum of all loop-lag observations, ms.
    pub lag_sum_ms: u64,
    /// Unix time (seconds) of the world loop's most recent iteration (0 =
    /// never). A liveness heartbeat — see [`Metrics::set_world_loop_iteration`].
    pub world_loop_last_iter_unix: i64,
    /// Scheduler entries pending.
    pub scheduler_queue_depth: i64,
    /// Room objects.
    pub rooms: i64,
    /// Exit objects.
    pub exits: i64,
    /// Thing objects.
    pub things: i64,
    /// Player objects.
    pub players: i64,
    /// NPC objects.
    pub npcs: i64,
    /// Store read-only mode.
    pub store_read_only: bool,
    /// Store write-queue depth.
    pub store_write_queue_depth: i64,
    /// Unix seconds of the last acked store write (0 = none yet).
    pub store_last_ack_unix: i64,
    /// NPCs with a live runtime.
    pub npc_active: i64,
    /// NPCs with an in-flight turn.
    pub npc_thinking: i64,
    /// Whether an LLM service is configured.
    pub llm_enabled: bool,
    /// Whether the global LLM kill switch is engaged.
    pub llm_globally_disabled: bool,
    /// Cumulative LLM counters by (feature, provider).
    pub llm_features: Vec<LlmFeatureStat>,
    /// Today's global LLM spend.
    pub llm_today: LlmTodaySpend,
}

impl MetricsSnapshot {
    /// The approximate p99 loop lag (ms) from the histogram: the upper bound
    /// of the bucket in which the 99th-percentile observation falls. `0` when
    /// nothing has been observed. Falls back to the max when the p99 lands in
    /// the open-topped `+Inf` bucket.
    #[must_use]
    pub fn loop_lag_p99_ms(&self) -> u64 {
        if self.lag_count == 0 {
            return 0;
        }
        // rank of the p99 observation (1-based), ceil(0.99 * count).
        let target = self.lag_count.saturating_mul(99).div_ceil(100).max(1);
        for (i, &bound) in LAG_BUCKETS_MS.iter().enumerate() {
            if self.lag_buckets[i] >= target {
                return bound;
            }
        }
        // In the +Inf bucket: the best bound we have is the observed max.
        self.loop_lag_max_ms
    }

    /// Prometheus text exposition (`text/plain; version=0.0.4`). Only
    /// non-sensitive aggregates — never per-player or secret data.
    #[must_use]
    pub fn to_prometheus(&self, uptime_s: u64) -> String {
        let mut o = String::with_capacity(4096);

        metric(
            &mut o,
            "mush_uptime_seconds",
            "gauge",
            "Seconds since the web server started.",
            &[(&[], uptime_s as f64)],
        );

        // Connections by transport.
        gauge_labeled(
            &mut o,
            "mush_connections",
            "Live transport connections by transport.",
            &[
                ("ssh", self.ssh_current as f64),
                ("web", self.web_current as f64),
            ],
        );
        counter_labeled(
            &mut o,
            "mush_connections_opened_total",
            "Transport connections accepted since boot, by transport.",
            &[
                ("ssh", self.ssh_open_total as f64),
                ("web", self.web_open_total as f64),
            ],
        );

        metric(
            &mut o,
            "mush_sessions",
            "gauge",
            "Live player sessions.",
            &[(&[], self.sessions_current as f64)],
        );
        metric(
            &mut o,
            "mush_commands_total",
            "counter",
            "Commands processed by the world task since boot.",
            &[(&[], self.commands_total as f64)],
        );

        // Loop lag: last/max gauges + a native histogram.
        metric(
            &mut o,
            "mush_loop_lag_last_ms",
            "gauge",
            "Most recent world-loop iteration lag, milliseconds.",
            &[(&[], self.loop_lag_last_ms as f64)],
        );
        metric(
            &mut o,
            "mush_loop_lag_max_ms",
            "gauge",
            "Worst world-loop iteration lag since boot, milliseconds.",
            &[(&[], self.loop_lag_max_ms as f64)],
        );
        self.write_lag_histogram(&mut o);

        // World-loop liveness heartbeat: advances every iteration. A wedged
        // loop freezes this while `loop_lag_last_ms` stays low — alert on
        // staleness (`time() - mush_world_loop_last_iteration_unixtime`).
        metric(
            &mut o,
            "mush_world_loop_last_iteration_unixtime",
            "gauge",
            "Unix time the world loop last began an iteration (0 = never); a liveness heartbeat.",
            &[(&[], self.world_loop_last_iter_unix as f64)],
        );

        metric(
            &mut o,
            "mush_scheduler_queue_depth",
            "gauge",
            "Pending scheduler entries (waits, module ticks).",
            &[(&[], self.scheduler_queue_depth as f64)],
        );

        // Object counts by kind.
        gauge_labeled(
            &mut o,
            "mush_objects",
            "World objects by kind.",
            &[
                ("room", self.rooms as f64),
                ("exit", self.exits as f64),
                ("thing", self.things as f64),
                ("player", self.players as f64),
                ("npc", self.npcs as f64),
            ],
        );

        // Store health.
        metric(
            &mut o,
            "mush_store_read_only",
            "gauge",
            "1 when the store is in read-only mode (writes failing), else 0.",
            &[(&[], if self.store_read_only { 1.0 } else { 0.0 })],
        );
        metric(
            &mut o,
            "mush_store_write_queue_depth",
            "gauge",
            "Store write batches queued and not yet acknowledged.",
            &[(&[], self.store_write_queue_depth as f64)],
        );
        metric(
            &mut o,
            "mush_store_last_ack_unixtime",
            "gauge",
            "Unix time of the last acknowledged store write (0 = none yet).",
            &[(&[], self.store_last_ack_unix as f64)],
        );

        // NPC agents.
        metric(
            &mut o,
            "mush_npc_active",
            "gauge",
            "NPC agents with a live runtime.",
            &[(&[], self.npc_active as f64)],
        );
        metric(
            &mut o,
            "mush_npc_thinking",
            "gauge",
            "NPC agents with an in-flight LLM turn.",
            &[(&[], self.npc_thinking as f64)],
        );

        // LLM aggregates (keyed only by feature route + provider — never
        // per-player, never a key or prompt).
        self.write_llm(&mut o);

        o
    }

    fn write_lag_histogram(&self, o: &mut String) {
        o.push_str("# HELP mush_loop_lag_ms World-loop iteration lag histogram, milliseconds.\n");
        o.push_str("# TYPE mush_loop_lag_ms histogram\n");
        // Each bucket already holds the cumulative count of observations
        // `<= le` (observe_loop_lag increments every bucket whose bound the
        // sample fits), which is exactly the Prometheus bucket value.
        for (i, &bound) in LAG_BUCKETS_MS.iter().enumerate() {
            o.push_str(&format!(
                "mush_loop_lag_ms_bucket{{le=\"{bound}\"}} {}\n",
                self.lag_buckets[i]
            ));
        }
        o.push_str(&format!(
            "mush_loop_lag_ms_bucket{{le=\"+Inf\"}} {}\n",
            self.lag_count
        ));
        o.push_str(&format!("mush_loop_lag_ms_sum {}\n", self.lag_sum_ms));
        o.push_str(&format!("mush_loop_lag_ms_count {}\n", self.lag_count));
    }

    fn write_llm(&self, o: &mut String) {
        metric(
            o,
            "mush_llm_enabled",
            "gauge",
            "1 when an LLM service is configured, else 0.",
            &[(&[], if self.llm_enabled { 1.0 } else { 0.0 })],
        );
        metric(
            o,
            "mush_llm_globally_disabled",
            "gauge",
            "1 when the global LLM kill switch is engaged, else 0.",
            &[(&[], if self.llm_globally_disabled { 1.0 } else { 0.0 })],
        );
        // Today's global spend (a gauge — resets at UTC midnight).
        metric(
            o,
            "mush_llm_today_tokens",
            "gauge",
            "LLM tokens consumed today (global), by direction.",
            &[
                (&[("dir", "input")], self.llm_today.input_tokens as f64),
                (&[("dir", "output")], self.llm_today.output_tokens as f64),
            ],
        );
        metric(
            o,
            "mush_llm_today_cost_usd",
            "gauge",
            "LLM cost consumed today (global), USD.",
            &[(&[], self.llm_today.cost_usd)],
        );

        // Cumulative per-(feature, provider) counters.
        o.push_str("# HELP mush_llm_calls_total LLM calls since boot, by feature, provider, and outcome.\n");
        o.push_str("# TYPE mush_llm_calls_total counter\n");
        for f in &self.llm_features {
            let ok = f
                .calls
                .saturating_sub(f.refusals + f.budget_trips + f.disabled + f.errors);
            for (outcome, n) in [
                ("ok", ok),
                ("refused", f.refusals),
                ("budget", f.budget_trips),
                ("disabled", f.disabled),
                ("error", f.errors),
            ] {
                o.push_str(&format!(
                    "mush_llm_calls_total{{feature=\"{}\",provider=\"{}\",outcome=\"{}\"}} {n}\n",
                    esc(&f.feature),
                    esc(&f.provider),
                    outcome,
                ));
            }
        }
        o.push_str("# HELP mush_llm_tokens_total LLM tokens since boot, by feature, provider, and direction.\n");
        o.push_str("# TYPE mush_llm_tokens_total counter\n");
        for f in &self.llm_features {
            o.push_str(&format!(
                "mush_llm_tokens_total{{feature=\"{}\",provider=\"{}\",dir=\"input\"}} {}\n",
                esc(&f.feature),
                esc(&f.provider),
                f.input_tokens,
            ));
            o.push_str(&format!(
                "mush_llm_tokens_total{{feature=\"{}\",provider=\"{}\",dir=\"output\"}} {}\n",
                esc(&f.feature),
                esc(&f.provider),
                f.output_tokens,
            ));
        }
        o.push_str(
            "# HELP mush_llm_cost_usd_total LLM cost since boot, by feature and provider, USD.\n",
        );
        o.push_str("# TYPE mush_llm_cost_usd_total counter\n");
        for f in &self.llm_features {
            o.push_str(&format!(
                "mush_llm_cost_usd_total{{feature=\"{}\",provider=\"{}\"}} {}\n",
                esc(&f.feature),
                esc(&f.provider),
                f.cost_usd,
            ));
        }
    }
}

/// Escape a Prometheus label value (`\`, `"`, and newline — the only three
/// characters the exposition format requires escaping). Feature and provider
/// ids are ASCII identifiers in practice, but escape defensively.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

/// Emit one metric family: a HELP+TYPE header then one line per label set.
/// `samples` pairs a label slice with its value.
fn metric(o: &mut String, name: &str, kind: &str, help: &str, samples: &[(&[(&str, &str)], f64)]) {
    o.push_str(&format!("# HELP {name} {help}\n"));
    o.push_str(&format!("# TYPE {name} {kind}\n"));
    for (labels, value) in samples {
        o.push_str(name);
        write_labels(o, labels);
        o.push(' ');
        o.push_str(&fmt_value(*value));
        o.push('\n');
    }
}

/// A single-label gauge family (the common `{transport|dir|kind=...}` case).
fn gauge_labeled(o: &mut String, name: &str, help: &str, samples: &[(&str, f64)]) {
    o.push_str(&format!("# HELP {name} {help}\n"));
    o.push_str(&format!("# TYPE {name} gauge\n"));
    // The label key is derived from the metric: connections/opened → transport,
    // objects → kind. Callers pass the value already; we tag with a generic
    // "id"-free scheme by reusing the family's natural label below.
    for (label, value) in samples {
        o.push_str(name);
        o.push_str(&format!("{{{}}}", label_kv(name, label)));
        o.push(' ');
        o.push_str(&fmt_value(*value));
        o.push('\n');
    }
}

fn counter_labeled(o: &mut String, name: &str, help: &str, samples: &[(&str, f64)]) {
    o.push_str(&format!("# HELP {name} {help}\n"));
    o.push_str(&format!("# TYPE {name} counter\n"));
    for (label, value) in samples {
        o.push_str(name);
        o.push_str(&format!("{{{}}}", label_kv(name, label)));
        o.push(' ');
        o.push_str(&fmt_value(*value));
        o.push('\n');
    }
}

/// Pick the label key for the single-label families.
fn label_kv(name: &str, value: &str) -> String {
    let key = if name.starts_with("mush_objects") {
        "kind"
    } else {
        "transport"
    };
    format!("{key}=\"{}\"", esc(value))
}

fn write_labels(o: &mut String, labels: &[(&str, &str)]) {
    if labels.is_empty() {
        return;
    }
    o.push('{');
    for (i, (k, v)) in labels.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        o.push_str(k);
        o.push_str("=\"");
        o.push_str(&esc(v));
        o.push('"');
    }
    o.push('}');
}

/// Format a value the Prometheus way: integers without a decimal point, and
/// finite floats with enough precision for cost figures.
fn fmt_value(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_counters_balance() {
        let m = Metrics::new();
        m.ssh_connection_opened();
        m.ssh_connection_opened();
        m.web_connection_opened();
        m.ssh_connection_closed();
        let s = m.snapshot();
        assert_eq!(s.ssh_open_total, 2);
        assert_eq!(s.ssh_current, 1);
        assert_eq!(s.web_open_total, 1);
        assert_eq!(s.web_current, 1);
        assert_eq!(m.connections_current(), 2);
    }

    #[test]
    fn command_counter_increments() {
        let m = Metrics::new();
        for _ in 0..5 {
            m.command_processed();
        }
        assert_eq!(m.snapshot().commands_total, 5);
    }

    #[test]
    fn loop_lag_last_max_and_histogram() {
        let m = Metrics::new();
        for ms in [3u64, 7, 40, 4, 900] {
            m.observe_loop_lag(ms);
        }
        let s = m.snapshot();
        assert_eq!(s.loop_lag_last_ms, 900);
        assert_eq!(s.loop_lag_max_ms, 900);
        assert_eq!(s.lag_count, 5);
        assert_eq!(s.lag_sum_ms, 3 + 7 + 40 + 4 + 900);
        // le=5 bucket holds {3,4} = 2; le=10 holds {3,4,7} = 3.
        let le5 = LAG_BUCKETS_MS.iter().position(|&b| b == 5).unwrap();
        let le10 = LAG_BUCKETS_MS.iter().position(|&b| b == 10).unwrap();
        assert_eq!(s.lag_buckets[le5], 2);
        assert_eq!(s.lag_buckets[le10], 3);
    }

    #[test]
    fn p99_picks_the_crossing_bucket() {
        let m = Metrics::new();
        // 99 fast (1ms) and 1 slow (900ms): p99 (rank 99) is still 1ms.
        for _ in 0..99 {
            m.observe_loop_lag(1);
        }
        m.observe_loop_lag(900);
        assert_eq!(m.snapshot().loop_lag_p99_ms(), 1);
        // 50/50: rank ceil(0.99*100)=99 crosses into the slow side.
        let m = Metrics::new();
        for _ in 0..50 {
            m.observe_loop_lag(1);
        }
        for _ in 0..50 {
            m.observe_loop_lag(900);
        }
        assert_eq!(m.snapshot().loop_lag_p99_ms(), 1000);
        // Empty: zero.
        assert_eq!(Metrics::new().snapshot().loop_lag_p99_ms(), 0);
    }

    #[test]
    fn health_json_shape() {
        let m = Metrics::new();
        m.ssh_connection_opened();
        m.observe_loop_lag(12);
        m.set_world_loop_iteration(1_700_000_000);
        m.publish_world(WorldSample {
            store_read_only: false,
            ..WorldSample::default()
        });
        // Read lock-free, straight from the atomics (no snapshot/clone).
        let j = m.health_json(42, "9.9.9");
        assert_eq!(j["ok"], true);
        assert_eq!(j["uptime_s"], 42);
        assert_eq!(j["version"], "9.9.9");
        assert_eq!(j["connections"], 1);
        assert_eq!(j["loop_lag_ms"], 12);
        assert_eq!(j["world_loop_unixtime"], 1_700_000_000);
        assert_eq!(j["store_ok"], true);
    }

    #[test]
    fn world_loop_heartbeat_is_published() {
        let m = Metrics::new();
        m.set_world_loop_iteration(1_700_000_123);
        assert_eq!(m.world_loop_last_iter_unix(), 1_700_000_123);
        let text = m.snapshot().to_prometheus(1);
        assert!(text.contains("mush_world_loop_last_iteration_unixtime 1700000123"));
    }

    #[test]
    fn prometheus_shape_and_no_secret_labels() {
        let m = Metrics::new();
        m.ssh_connection_opened();
        m.web_connection_opened();
        m.command_processed();
        m.observe_loop_lag(4);
        m.publish_world(WorldSample {
            sessions_current: 2,
            connections_current: 3,
            scheduler_queue_depth: 1,
            rooms: 5,
            exits: 4,
            things: 2,
            players: 2,
            npcs: 1,
            store_read_only: false,
            npc_active: 1,
            npc_thinking: 0,
        });
        m.publish_llm(
            true,
            false,
            vec![LlmFeatureStat {
                feature: "worldgen_room".into(),
                provider: "openrouter".into(),
                calls: 3,
                refusals: 1,
                budget_trips: 0,
                disabled: 0,
                errors: 0,
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.0123,
            }],
            LlmTodaySpend {
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.0123,
            },
        );
        let text = m.snapshot().to_prometheus(60);

        // Shape: HELP/TYPE headers present, labeled families rendered.
        assert!(text.contains("# TYPE mush_connections gauge"));
        assert!(text.contains("mush_connections{transport=\"ssh\"} 1"));
        assert!(text.contains("mush_connections{transport=\"web\"} 1"));
        assert!(text.contains("mush_commands_total 1"));
        assert!(text.contains("# TYPE mush_loop_lag_ms histogram"));
        assert!(text.contains("mush_loop_lag_ms_bucket{le=\"5\"} 1"));
        assert!(text.contains("mush_loop_lag_ms_count 1"));
        assert!(text.contains("mush_objects{kind=\"room\"} 5"));
        assert!(text.contains("mush_store_read_only 0"));
        assert!(text.contains("mush_npc_active 1"));
        assert!(text.contains(
            "mush_llm_calls_total{feature=\"worldgen_room\",provider=\"openrouter\",outcome=\"ok\"} 2"
        ));
        assert!(text.contains(
            "mush_llm_calls_total{feature=\"worldgen_room\",provider=\"openrouter\",outcome=\"refused\"} 1"
        ));
        assert!(text.contains(
            "mush_llm_cost_usd_total{feature=\"worldgen_room\",provider=\"openrouter\"} 0.0123"
        ));

        // No secret / per-player labels anywhere. (The substring "token"
        // appears legitimately in aggregate metric NAMES like
        // `mush_llm_tokens_total`, so the guard targets secret-shaped label
        // keys and credential material, not that word.)
        for forbidden in [
            "token=\"",
            "password",
            "Bearer",
            "account=\"",
            "player=\"",
            "ip=\"",
            "actor=\"",
            "secret",
        ] {
            assert!(
                !text.contains(forbidden),
                "metrics text must not contain {forbidden:?}"
            );
        }
    }
}
