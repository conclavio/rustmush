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

/// Authoritative-bundle size histogram bucket upper bounds, bytes (H3). The
/// top bucket is the kernel's `MAX_AUTHORITATIVE_BUNDLE_BYTES` (16 MiB), so a
/// bundle that lands there is at the cap.
pub const BUNDLE_BUCKETS_BYTES: [u64; 11] = [
    1 << 10,
    4 << 10,
    16 << 10,
    64 << 10,
    256 << 10,
    1 << 20,
    2 << 20,
    4 << 20,
    8 << 20,
    12 << 20,
    16 << 20,
];

/// Authoritative-bundle replay-write count histogram bucket upper bounds
/// (H3). The top bucket is the kernel's `MAX_AUTHORITATIVE_REPLAY_WRITES`.
pub const WRITE_BUCKETS: [u64; 11] = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024];

/// How one authoritative commit ended, for the `outcome` label of
/// `mush_authoritative_commits_total` (H3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthoritativeCommitOutcome {
    /// The exact receipt was finalized and checkpointed.
    Finalized,
    /// Reservation definitively created no intent (a refusal before the slot
    /// was held).
    Refused,
    /// An integrity failure: an intent may exist, identity is contradictory,
    /// or the store is damaged.
    Integrity,
}

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
    /// Panics caught at the world task's dispatch boundary. Any value
    /// above zero means the single writer was thrown out of its own state
    /// machine and the world is frozen; it is an alert, not a trend.
    world_panics_total: AtomicU64,

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
    integrity_frozen: AtomicBool,
    store_write_queue_depth: AtomicI64,
    store_legacy_queue_depth: AtomicI64,
    store_authoritative_queue_depth: AtomicI64,
    store_control_queue_depth: AtomicI64,
    store_last_ack_unix: AtomicI64,

    // -- the authoritative lane (H3; event-driven from the store writer) --
    /// The single pending-intent slot: held from the exact reservation
    /// receipt until the commit settles (finalized, failed or quarantined).
    auth_slot_held: AtomicBool,
    /// Unix seconds the slot was last taken (0 = never).
    auth_slot_held_since_unix: AtomicI64,
    auth_hold_last_ms: AtomicU64,
    auth_hold_max_ms: AtomicU64,
    auth_hold_buckets: [AtomicU64; 12],
    auth_hold_count: AtomicU64,
    auth_hold_sum_ms: AtomicU64,
    auth_bytes_last: AtomicU64,
    auth_bytes_buckets: [AtomicU64; 11],
    auth_bytes_count: AtomicU64,
    auth_bytes_sum: AtomicU64,
    auth_writes_last: AtomicU64,
    auth_writes_buckets: [AtomicU64; 11],
    auth_writes_count: AtomicU64,
    auth_writes_sum: AtomicU64,
    auth_commits_finalized_total: AtomicU64,
    auth_commits_refused_total: AtomicU64,
    auth_commits_integrity_total: AtomicU64,
    auth_commits_quarantined_total: AtomicU64,

    // -- NPC agents (published each sample) --
    npc_active: AtomicI64,
    npc_thinking: AtomicI64,
    npc_decisions_ready: AtomicI64,
    npc_decisions_leased: AtomicI64,
    npc_decisions_resolving: AtomicI64,
    // J1 (doc 12 D-12-1 / D-12-3): the economy journal and its last replay.
    economy_journal_entries: AtomicI64,
    economy_journal_per_hour: AtomicI64,
    economy_replay_us_per_entry: AtomicI64,
    economy_replay_ms: AtomicI64,
    economy_fetch_ms: AtomicI64,
    // J5 (doc 12 section 10): the ledger archive tier.
    ledger_archive_chunks: AtomicI64,
    ledger_archive_purged_chunks: AtomicI64,
    ledger_resident_bytes: AtomicI64,
    ledger_archive_last_pass_unix: AtomicI64,
    ledger_archive_last_pass_ok: AtomicI64,

    // -- IPC boundary bounding (M18; event-driven, always live) --
    /// Input frames (`Cmd`/`Query`) shed gateway->server when `to_server` is
    /// full (monotonic counter).
    ipc_frames_shed_total: AtomicU64,
    /// IPC write_frame calls that timed out on a wedged peer, either direction
    /// (monotonic counter).
    ipc_write_timeouts_total: AtomicU64,
    /// Server->gateway (`to_gw`) overflow events: a must-deliver frame hit a
    /// full queue and forced a whole-boundary teardown (never a per-message
    /// drop). Monotonic counter — the surfaced signal for the teardown class
    /// that the write-timeout counter does not cover.
    ipc_to_gw_overflow_total: AtomicU64,
    /// Nonconforming `ReAnnounce` snapshots rejected atomically at the server
    /// boundary because they exceeded lifecycle custody. The boundary is torn
    /// down so the gateway-retained roster can retry; no partial snapshot is
    /// ever applied.
    ipc_reannounce_protocol_violations_total: AtomicU64,
    /// Coordinator claims the server had RETAINED (custody kept, no
    /// disposition published) and later reaped because they aged past the
    /// retention TTL. Monotonic counter — retention is silent by design, so
    /// this is the only operational signal that a boundary is accumulating
    /// requests whose durable fate nothing could prove.
    ipc_retained_claims_reaped_total: AtomicU64,
    /// Coordinator submissions whose mandatory read-only preflight stayed
    /// TRANSIENT to its bound, so the server deferred the work instead of
    /// spinning on the store: the split boundary retains custody for the
    /// gateway's next replay, the all-in-one handle answers `Busy`. Monotonic
    /// counter and the only cheap signal that the authoritative store is
    /// answering neither yes nor no; it climbing means players are being told
    /// "try again" by a store hiccup, not by a rule.
    coordinator_preflight_deferred_total: AtomicU64,
    /// High-water mark of the gateway->server (`to_server`) queue depth
    /// (`fetch_max` gauge).
    ipc_queue_high_water_to_server: AtomicU64,
    /// High-water mark of the server->gateway (`to_gw`) queue depth
    /// (`fetch_max` gauge).
    ipc_queue_high_water_to_gw: AtomicU64,

    // -- exact-receipt operational custody (event-driven, always live) --
    commit_pending_barrier_refused_total: AtomicU64,
    commit_pending_reattach_coalesced_total: AtomicU64,
    commit_pending_reattach_rejected_total: AtomicU64,
    commit_pending_barriers: AtomicU64,
    commit_pending_reattach_records: AtomicU64,
    commit_pending_operational_bytes: AtomicU64,
    /// Observability only until K2 preflight construction supplies a safe hard
    /// cap before legacy handlers mutate authoritative RAM.
    commit_pending_sends: AtomicU64,
    /// Observability only; see `commit_pending_sends`.
    commit_pending_effects: AtomicU64,

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

    /// Count one panic caught at the world task's dispatch boundary.
    ///
    /// The world task must never panic (docs/PLAN.md); when one is caught
    /// anyway the loop freezes persistence and stops. This counter is what
    /// makes that visible to a scraper without reading the log.
    pub fn world_panic(&self) {
        self.world_panics_total.fetch_add(1, Ordering::Relaxed);
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
            "integrity_ok": !self.integrity_frozen.load(Ordering::Relaxed),
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
        self.integrity_frozen
            .store(w.integrity_frozen, Ordering::Relaxed);
        self.npc_active.store(w.npc_active, Ordering::Relaxed);
        self.npc_thinking.store(w.npc_thinking, Ordering::Relaxed);
        self.npc_decisions_ready
            .store(w.npc_decisions_ready, Ordering::Relaxed);
        self.npc_decisions_leased
            .store(w.npc_decisions_leased, Ordering::Relaxed);
        self.npc_decisions_resolving
            .store(w.npc_decisions_resolving, Ordering::Relaxed);
        self.economy_journal_entries
            .store(w.economy_journal_entries, Ordering::Relaxed);
        self.economy_journal_per_hour
            .store(w.economy_journal_per_hour, Ordering::Relaxed);
        self.economy_replay_us_per_entry
            .store(w.economy_replay_us_per_entry, Ordering::Relaxed);
        self.economy_replay_ms
            .store(w.economy_replay_ms, Ordering::Relaxed);
        self.economy_fetch_ms
            .store(w.economy_fetch_ms, Ordering::Relaxed);
    }

    /// Publish what the ledger archive pass did (J5, doc 12 section 10): the
    /// archive job, after every pass and once at start.
    pub fn publish_ledger_archive(&self, s: LedgerArchiveSample) {
        self.ledger_archive_chunks
            .store(s.chunks, Ordering::Relaxed);
        self.ledger_archive_purged_chunks
            .store(s.purged_chunks, Ordering::Relaxed);
        self.ledger_resident_bytes
            .store(s.resident_bytes, Ordering::Relaxed);
        self.ledger_archive_last_pass_unix
            .store(s.last_pass_unix, Ordering::Relaxed);
        self.ledger_archive_last_pass_ok
            .store(i64::from(s.last_pass_ok), Ordering::Relaxed);
    }

    /// Publish the store write-queue depth (world task on each flush) and the
    /// last-ack timestamp (the writer task on each acked write).
    pub fn set_store_write_queue_depth(&self, depth: i64) {
        self.store_write_queue_depth.store(depth, Ordering::Relaxed);
    }

    /// Immediately publish persistence admission state. Unlike the aggregate
    /// world sample, mode transitions cannot be throttled: health checks must
    /// observe a read-only/frozen transition even when the world then waits on
    /// a stuck receipt and performs no later iteration.
    pub fn set_persistence_state(&self, store_read_only: bool, integrity_frozen: bool) {
        self.store_read_only
            .store(store_read_only, Ordering::Relaxed);
        self.integrity_frozen
            .store(integrity_frozen, Ordering::Relaxed);
    }

    /// Bump the store write-queue depth by `delta` (writer enqueue/ack).
    pub fn add_store_write_queue_depth(&self, delta: i64) {
        self.store_write_queue_depth
            .fetch_add(delta, Ordering::Relaxed);
    }

    /// Bump accepted/in-flight legacy writer work by `delta`.
    pub fn add_store_legacy_queue_depth(&self, delta: i64) {
        self.store_legacy_queue_depth
            .fetch_add(delta, Ordering::Relaxed);
    }

    /// Bump accepted/in-flight exact-token writer work by `delta`.
    pub fn add_store_authoritative_queue_depth(&self, delta: i64) {
        self.store_authoritative_queue_depth
            .fetch_add(delta, Ordering::Relaxed);
    }

    /// Bump accepted/in-flight persistence-control work by `delta`.
    pub fn add_store_control_queue_depth(&self, delta: i64) {
        self.store_control_queue_depth
            .fetch_add(delta, Ordering::Relaxed);
    }

    // -- the authoritative lane (H3) ---------------------------------------

    /// The store writer took the single pending-intent slot (the exact
    /// reservation receipt arrived) at `now_unix`.
    pub fn authoritative_slot_held(&self, now_unix: i64) {
        self.auth_slot_held.store(true, Ordering::Relaxed);
        self.auth_slot_held_since_unix
            .store(now_unix, Ordering::Relaxed);
    }

    /// The slot is free again (the commit settled either way).
    pub fn authoritative_slot_released(&self) {
        self.auth_slot_held.store(false, Ordering::Relaxed);
    }

    /// Record one settled authoritative commit: how long the slot was held
    /// (`None` for a commit refused before the slot was taken — a refusal
    /// is counted but never enters the hold histogram, so a refusal storm
    /// cannot drag the hold percentiles down), the bundle's canonical encoded
    /// size, its replay-write count, and how it ended. Aggregates only —
    /// never a token, hash, owner or key.
    pub fn observe_authoritative_commit(
        &self,
        hold_ms: Option<u64>,
        bundle_bytes: u64,
        replay_writes: u64,
        outcome: AuthoritativeCommitOutcome,
    ) {
        if let Some(hold_ms) = hold_ms {
            self.auth_hold_last_ms.store(hold_ms, Ordering::Relaxed);
            self.auth_hold_max_ms.fetch_max(hold_ms, Ordering::Relaxed);
            self.auth_hold_count.fetch_add(1, Ordering::Relaxed);
            self.auth_hold_sum_ms.fetch_add(hold_ms, Ordering::Relaxed);
            for (i, &bound) in LAG_BUCKETS_MS.iter().enumerate() {
                if hold_ms <= bound {
                    self.auth_hold_buckets[i].fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        self.auth_bytes_last.store(bundle_bytes, Ordering::Relaxed);
        self.auth_bytes_count.fetch_add(1, Ordering::Relaxed);
        self.auth_bytes_sum
            .fetch_add(bundle_bytes, Ordering::Relaxed);
        for (i, &bound) in BUNDLE_BUCKETS_BYTES.iter().enumerate() {
            if bundle_bytes <= bound {
                self.auth_bytes_buckets[i].fetch_add(1, Ordering::Relaxed);
            }
        }
        self.auth_writes_last
            .store(replay_writes, Ordering::Relaxed);
        self.auth_writes_count.fetch_add(1, Ordering::Relaxed);
        self.auth_writes_sum
            .fetch_add(replay_writes, Ordering::Relaxed);
        for (i, &bound) in WRITE_BUCKETS.iter().enumerate() {
            if replay_writes <= bound {
                self.auth_writes_buckets[i].fetch_add(1, Ordering::Relaxed);
            }
        }
        let counter = match outcome {
            AuthoritativeCommitOutcome::Finalized => &self.auth_commits_finalized_total,
            AuthoritativeCommitOutcome::Refused => &self.auth_commits_refused_total,
            AuthoritativeCommitOutcome::Integrity => &self.auth_commits_integrity_total,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// One authoritative bundle was spooled to the quarantine dead-letter
    /// (counted beside its `Integrity` outcome).
    pub fn authoritative_bundle_quarantined(&self) {
        self.auth_commits_quarantined_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record that the store acknowledged a write at `unix_s`.
    pub fn set_store_last_ack_unix(&self, unix_s: i64) {
        self.store_last_ack_unix.store(unix_s, Ordering::Relaxed);
    }

    // -- IPC boundary bounding (M18) --------------------------------------

    /// Count one input frame shed gateway->server (the `to_server` queue was
    /// full). Monotonic; the shed site also logs a warn.
    pub fn ipc_frame_shed(&self) {
        self.ipc_frames_shed_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Count one IPC `write_frame` timeout on a wedged peer (either writer).
    /// Monotonic; the timeout site also logs a warn and tears the boundary down.
    pub fn ipc_write_timeout(&self) {
        self.ipc_write_timeouts_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Count one server->gateway (`to_gw`) overflow: a must-deliver frame hit a
    /// full queue and forced a boundary teardown. Monotonic; the overflow site
    /// also logs a warn and notifies the writer to tear down.
    pub fn ipc_to_gw_overflow(&self) {
        self.ipc_to_gw_overflow_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Count one oversized `ReAnnounce` protocol violation. The server tears
    /// the boundary down before applying any record; the gateway remains owner
    /// of the full roster and retries it through the reconnect handshake.
    pub fn ipc_reannounce_protocol_violation(&self) {
        self.ipc_reannounce_protocol_violations_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Count `reaped` retained coordinator claims freed by one reaper pass.
    /// The reap publishes nothing to the gateway (which still holds its own
    /// replay copy), so this counter and one warn are the whole signal.
    pub fn ipc_retained_claims_reaped(&self, reaped: u64) {
        self.ipc_retained_claims_reaped_total
            .fetch_add(reaped, Ordering::Relaxed);
    }

    /// Count one coordinator submission deferred at the preflight bound: the
    /// store stayed transient for the whole bounded window, so the request was
    /// handed back to whoever still holds a copy of it rather than retried
    /// forever inside the handler.
    pub fn coordinator_preflight_deferred(&self) {
        self.coordinator_preflight_deferred_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record a gateway->server (`to_server`) queue occupancy observation,
    /// keeping the high-water mark (`fetch_max`).
    pub fn observe_ipc_to_server_depth(&self, depth: u64) {
        self.ipc_queue_high_water_to_server
            .fetch_max(depth, Ordering::Relaxed);
    }

    /// Record a server->gateway (`to_gw`) queue occupancy observation, keeping
    /// the high-water mark (`fetch_max`).
    pub fn observe_ipc_to_gw_depth(&self, depth: u64) {
        self.ipc_queue_high_water_to_gw
            .fetch_max(depth, Ordering::Relaxed);
    }

    /// Total input frames shed gateway->server since boot.
    #[must_use]
    pub fn ipc_frames_shed_total(&self) -> u64 {
        self.ipc_frames_shed_total.load(Ordering::Relaxed)
    }

    // -- exact-receipt operational custody -------------------------------

    /// Count one barrier waiter refused at the fixed pending-receipt cap.
    pub fn commit_pending_barrier_refused(&self) {
        self.commit_pending_barrier_refused_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Count one older full reattach snapshot replaced by a newer snapshot.
    pub fn commit_pending_reattach_coalesced(&self) {
        self.commit_pending_reattach_coalesced_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Count one reattach snapshot rejected atomically for violating the
    /// lifecycle-roster cap. This is an invariant/protocol violation, not
    /// ordinary load shedding.
    pub fn commit_pending_reattach_rejected(&self) {
        self.commit_pending_reattach_rejected_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Publish custody held behind the currently unresolved exact receipt.
    /// Barrier/reattach count and logical bytes have hard ceilings in the
    /// world task. Sends/effects are exposed to make the legacy pre-K2 gap
    /// visible; they are not silently truncated after a handler has mutated
    /// authoritative RAM.
    pub fn set_commit_pending_custody(
        &self,
        barriers: u64,
        reattach_records: u64,
        operational_bytes: u64,
        sends: u64,
        effects: u64,
    ) {
        self.commit_pending_barriers
            .store(barriers, Ordering::Relaxed);
        self.commit_pending_reattach_records
            .store(reattach_records, Ordering::Relaxed);
        self.commit_pending_operational_bytes
            .store(operational_bytes, Ordering::Relaxed);
        self.commit_pending_sends.store(sends, Ordering::Relaxed);
        self.commit_pending_effects
            .store(effects, Ordering::Relaxed);
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

    /// Whether no authoritative-integrity incident is latched.
    #[must_use]
    pub fn integrity_ok(&self) -> bool {
        !self.integrity_frozen.load(Ordering::Relaxed)
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
            world_panics_total: self.world_panics_total.load(Ordering::Relaxed),
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
            integrity_frozen: self.integrity_frozen.load(Ordering::Relaxed),
            store_write_queue_depth: self.store_write_queue_depth.load(Ordering::Relaxed),
            store_legacy_queue_depth: self.store_legacy_queue_depth.load(Ordering::Relaxed),
            store_authoritative_queue_depth: self
                .store_authoritative_queue_depth
                .load(Ordering::Relaxed),
            store_control_queue_depth: self.store_control_queue_depth.load(Ordering::Relaxed),
            store_last_ack_unix: self.store_last_ack_unix.load(Ordering::Relaxed),
            auth_slot_held: self.auth_slot_held.load(Ordering::Relaxed),
            auth_slot_held_since_unix: self.auth_slot_held_since_unix.load(Ordering::Relaxed),
            auth_hold_last_ms: self.auth_hold_last_ms.load(Ordering::Relaxed),
            auth_hold_max_ms: self.auth_hold_max_ms.load(Ordering::Relaxed),
            auth_hold_buckets: std::array::from_fn(|i| {
                self.auth_hold_buckets[i].load(Ordering::Relaxed)
            }),
            auth_hold_count: self.auth_hold_count.load(Ordering::Relaxed),
            auth_hold_sum_ms: self.auth_hold_sum_ms.load(Ordering::Relaxed),
            auth_bytes_last: self.auth_bytes_last.load(Ordering::Relaxed),
            auth_bytes_buckets: std::array::from_fn(|i| {
                self.auth_bytes_buckets[i].load(Ordering::Relaxed)
            }),
            auth_bytes_count: self.auth_bytes_count.load(Ordering::Relaxed),
            auth_bytes_sum: self.auth_bytes_sum.load(Ordering::Relaxed),
            auth_writes_last: self.auth_writes_last.load(Ordering::Relaxed),
            auth_writes_buckets: std::array::from_fn(|i| {
                self.auth_writes_buckets[i].load(Ordering::Relaxed)
            }),
            auth_writes_count: self.auth_writes_count.load(Ordering::Relaxed),
            auth_writes_sum: self.auth_writes_sum.load(Ordering::Relaxed),
            auth_commits_finalized_total: self.auth_commits_finalized_total.load(Ordering::Relaxed),
            auth_commits_refused_total: self.auth_commits_refused_total.load(Ordering::Relaxed),
            auth_commits_integrity_total: self.auth_commits_integrity_total.load(Ordering::Relaxed),
            auth_commits_quarantined_total: self
                .auth_commits_quarantined_total
                .load(Ordering::Relaxed),
            npc_active: self.npc_active.load(Ordering::Relaxed),
            npc_thinking: self.npc_thinking.load(Ordering::Relaxed),
            npc_decisions_ready: self.npc_decisions_ready.load(Ordering::Relaxed),
            npc_decisions_leased: self.npc_decisions_leased.load(Ordering::Relaxed),
            npc_decisions_resolving: self.npc_decisions_resolving.load(Ordering::Relaxed),
            economy_journal_entries: self.economy_journal_entries.load(Ordering::Relaxed),
            economy_journal_per_hour: self.economy_journal_per_hour.load(Ordering::Relaxed),
            economy_replay_us_per_entry: self.economy_replay_us_per_entry.load(Ordering::Relaxed),
            economy_replay_ms: self.economy_replay_ms.load(Ordering::Relaxed),
            economy_fetch_ms: self.economy_fetch_ms.load(Ordering::Relaxed),
            ledger_archive_chunks: self.ledger_archive_chunks.load(Ordering::Relaxed),
            ledger_archive_purged_chunks: self.ledger_archive_purged_chunks.load(Ordering::Relaxed),
            ledger_resident_bytes: self.ledger_resident_bytes.load(Ordering::Relaxed),
            ledger_archive_last_pass_unix: self
                .ledger_archive_last_pass_unix
                .load(Ordering::Relaxed),
            ledger_archive_last_pass_ok: self.ledger_archive_last_pass_ok.load(Ordering::Relaxed),
            ipc_frames_shed_total: self.ipc_frames_shed_total.load(Ordering::Relaxed),
            ipc_write_timeouts_total: self.ipc_write_timeouts_total.load(Ordering::Relaxed),
            ipc_to_gw_overflow_total: self.ipc_to_gw_overflow_total.load(Ordering::Relaxed),
            ipc_reannounce_protocol_violations_total: self
                .ipc_reannounce_protocol_violations_total
                .load(Ordering::Relaxed),
            ipc_retained_claims_reaped_total: self
                .ipc_retained_claims_reaped_total
                .load(Ordering::Relaxed),
            coordinator_preflight_deferred_total: self
                .coordinator_preflight_deferred_total
                .load(Ordering::Relaxed),
            ipc_queue_high_water_to_server: self
                .ipc_queue_high_water_to_server
                .load(Ordering::Relaxed),
            ipc_queue_high_water_to_gw: self.ipc_queue_high_water_to_gw.load(Ordering::Relaxed),
            commit_pending_barrier_refused_total: self
                .commit_pending_barrier_refused_total
                .load(Ordering::Relaxed),
            commit_pending_reattach_coalesced_total: self
                .commit_pending_reattach_coalesced_total
                .load(Ordering::Relaxed),
            commit_pending_reattach_rejected_total: self
                .commit_pending_reattach_rejected_total
                .load(Ordering::Relaxed),
            commit_pending_barriers: self.commit_pending_barriers.load(Ordering::Relaxed),
            commit_pending_reattach_records: self
                .commit_pending_reattach_records
                .load(Ordering::Relaxed),
            commit_pending_operational_bytes: self
                .commit_pending_operational_bytes
                .load(Ordering::Relaxed),
            commit_pending_sends: self.commit_pending_sends.load(Ordering::Relaxed),
            commit_pending_effects: self.commit_pending_effects.load(Ordering::Relaxed),
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
    /// Whether authoritative integrity is frozen pending recovery.
    pub integrity_frozen: bool,
    /// NPCs with a live runtime (stimulated at least once).
    pub npc_active: i64,
    /// NPCs with an in-flight LLM turn.
    pub npc_thinking: i64,
    /// H2a: durable NPC decisions in `Ready`.
    pub npc_decisions_ready: i64,
    /// H2a: durable NPC decisions in `Leased`.
    pub npc_decisions_leased: i64,
    /// H2a: durable NPC decisions in `Resolving` (a fallback awaiting its
    /// action counts here too).
    pub npc_decisions_resolving: i64,
    /// J1 (doc 12 D-12-1): journal transactions the economy holds.
    pub economy_journal_entries: i64,
    /// J1: journal transactions posted per hour since the economy booted —
    /// the growth rate the checkpoint decision keys on.
    pub economy_journal_per_hour: i64,
    /// J1: the last cold replay's cost per entry, microseconds.
    pub economy_replay_us_per_entry: i64,
    /// J1: the last cold replay, milliseconds (the C17 timer).
    pub economy_replay_ms: i64,
    /// J1 (D-12-3): the last boot's economy namespace fetch, milliseconds.
    pub economy_fetch_ms: i64,
}

/// What the ledger archive pass publishes (J5, doc 12 section 10).
#[derive(Clone, Copy, Debug, Default)]
pub struct LedgerArchiveSample {
    /// Chunks the archive holds.
    pub chunks: i64,
    /// Of those, chunks the primary has been purged of.
    pub purged_chunks: i64,
    /// Ledger bytes resident in the primary after the last pass (-1 before one).
    pub resident_bytes: i64,
    /// When the last pass ran (unix seconds; 0 before one).
    pub last_pass_unix: i64,
    /// Whether the last pass finished without a warning.
    pub last_pass_ok: bool,
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
    /// Panics caught at the world task's dispatch boundary since boot.
    pub world_panics_total: u64,
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
    /// Authoritative integrity freeze state.
    pub integrity_frozen: bool,
    /// Store write-queue depth.
    pub store_write_queue_depth: i64,
    /// Accepted/in-flight legacy write batches.
    pub store_legacy_queue_depth: i64,
    /// Accepted/in-flight exact-token authoritative bundles.
    pub store_authoritative_queue_depth: i64,
    /// Accepted/in-flight persistence-control jobs.
    pub store_control_queue_depth: i64,
    /// Unix seconds of the last acked store write (0 = none yet).
    pub store_last_ack_unix: i64,
    /// Whether the single authoritative pending-intent slot is held now (H3).
    pub auth_slot_held: bool,
    /// Unix seconds the slot was last taken (0 = never).
    pub auth_slot_held_since_unix: i64,
    /// Most recent slot hold, ms.
    pub auth_hold_last_ms: u64,
    /// Longest slot hold since boot, ms.
    pub auth_hold_max_ms: u64,
    /// Cumulative slot-hold histogram bucket counts (aligned to [`LAG_BUCKETS_MS`]).
    pub auth_hold_buckets: [u64; 12],
    /// Commits that held the slot (the hold histogram's count; refusals
    /// never held it and are not counted here).
    pub auth_hold_count: u64,
    /// Sum of all slot holds, ms.
    pub auth_hold_sum_ms: u64,
    /// Most recent bundle's canonical encoded size, bytes.
    pub auth_bytes_last: u64,
    /// Cumulative bundle-size histogram bucket counts (aligned to [`BUNDLE_BUCKETS_BYTES`]).
    pub auth_bytes_buckets: [u64; 11],
    /// Bundle-size observations.
    pub auth_bytes_count: u64,
    /// Sum of all bundle sizes, bytes.
    pub auth_bytes_sum: u64,
    /// Most recent bundle's replay-write count.
    pub auth_writes_last: u64,
    /// Cumulative replay-write histogram bucket counts (aligned to [`WRITE_BUCKETS`]).
    pub auth_writes_buckets: [u64; 11],
    /// Replay-write observations.
    pub auth_writes_count: u64,
    /// Sum of all replay-write counts.
    pub auth_writes_sum: u64,
    /// Commits finalized since boot.
    pub auth_commits_finalized_total: u64,
    /// Commits refused before the slot was taken since boot.
    pub auth_commits_refused_total: u64,
    /// Commits that ended in an integrity failure since boot.
    pub auth_commits_integrity_total: u64,
    /// Bundles spooled to the quarantine dead-letter since boot.
    pub auth_commits_quarantined_total: u64,
    /// NPCs with a live runtime.
    pub npc_active: i64,
    /// NPCs with an in-flight turn.
    pub npc_thinking: i64,
    /// H2a: durable NPC decisions in `Ready`.
    pub npc_decisions_ready: i64,
    /// H2a: durable NPC decisions in `Leased`.
    pub npc_decisions_leased: i64,
    /// H2a: durable NPC decisions in `Resolving` (a fallback awaiting its
    /// action counts here too).
    pub npc_decisions_resolving: i64,
    /// J1 (doc 12 D-12-1): journal transactions the economy holds.
    pub economy_journal_entries: i64,
    /// J1: journal transactions posted per hour since the economy booted —
    /// the growth rate the checkpoint decision keys on.
    pub economy_journal_per_hour: i64,
    /// J1: the last cold replay's cost per entry, microseconds.
    pub economy_replay_us_per_entry: i64,
    /// J1: the last cold replay, milliseconds (the C17 timer).
    pub economy_replay_ms: i64,
    /// J1 (D-12-3): the last boot's economy namespace fetch, milliseconds.
    pub economy_fetch_ms: i64,
    /// J5: chunks the ledger archive holds.
    pub ledger_archive_chunks: i64,
    /// J5: of those, chunks the primary has been purged of.
    pub ledger_archive_purged_chunks: i64,
    /// J5: ledger bytes resident in the primary after the last pass (-1 before one).
    pub ledger_resident_bytes: i64,
    /// J5: when the last archive pass ran (unix seconds; 0 before one).
    pub ledger_archive_last_pass_unix: i64,
    /// J5: whether the last pass finished without a warning (0/1).
    pub ledger_archive_last_pass_ok: i64,
    /// Input frames shed gateway->server since boot (M18).
    pub ipc_frames_shed_total: u64,
    /// IPC write timeouts on a wedged peer since boot (M18).
    pub ipc_write_timeouts_total: u64,
    /// Server->gateway (`to_gw`) overflow teardowns since boot (M18).
    pub ipc_to_gw_overflow_total: u64,
    /// Oversized `ReAnnounce` protocol violations since boot.
    pub ipc_reannounce_protocol_violations_total: u64,
    /// Retained coordinator claims reaped at the retention TTL since boot.
    pub ipc_retained_claims_reaped_total: u64,
    /// Coordinator submissions deferred at the preflight bound since boot: a
    /// store that answered transiently for the whole bounded window.
    pub coordinator_preflight_deferred_total: u64,
    /// High-water mark of the gateway->server (`to_server`) queue depth (M18).
    pub ipc_queue_high_water_to_server: u64,
    /// High-water mark of the server->gateway (`to_gw`) queue depth (M18).
    pub ipc_queue_high_water_to_gw: u64,
    /// Barrier waiters refused at the pending-receipt cap since boot.
    pub commit_pending_barrier_refused_total: u64,
    /// Older full reattach snapshots coalesced since boot.
    pub commit_pending_reattach_coalesced_total: u64,
    /// Reattach snapshots atomically rejected for violating the roster cap.
    pub commit_pending_reattach_rejected_total: u64,
    /// Barrier waiters held behind the unresolved exact receipt.
    pub commit_pending_barriers: u64,
    /// Records in the one coalesced reattach snapshot.
    pub commit_pending_reattach_records: u64,
    /// Conservative logical bytes of bounded barrier/reattach custody.
    pub commit_pending_operational_bytes: u64,
    /// Resolved sends held behind the receipt (observable; K2 bounds construction).
    pub commit_pending_sends: u64,
    /// External effects held behind the receipt (observable; K2 bounds construction).
    pub commit_pending_effects: u64,
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
        histogram_p99(
            &LAG_BUCKETS_MS,
            &self.lag_buckets,
            self.lag_count,
            self.loop_lag_max_ms,
        )
    }

    /// The approximate p99 authoritative slot hold (ms), by the same rule as
    /// [`MetricsSnapshot::loop_lag_p99_ms`] (H3). `0` before any commit.
    #[must_use]
    pub fn authoritative_hold_p99_ms(&self) -> u64 {
        histogram_p99(
            &LAG_BUCKETS_MS,
            &self.auth_hold_buckets,
            self.auth_hold_count,
            self.auth_hold_max_ms,
        )
    }

    /// The mean authoritative slot hold (ms); `0` before any commit.
    #[must_use]
    pub fn authoritative_hold_mean_ms(&self) -> u64 {
        self.auth_hold_sum_ms
            .checked_div(self.auth_hold_count)
            .unwrap_or(0)
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
        metric(
            &mut o,
            "mush_world_panics_total",
            "counter",
            "Panics caught at the world task's dispatch boundary since boot.",
            &[(&[], self.world_panics_total as f64)],
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
        self.write_authoritative_lane(&mut o);

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
            "mush_integrity_frozen",
            "gauge",
            "1 when an authoritative-integrity incident is latched pending recovery, else 0.",
            &[(&[], if self.integrity_frozen { 1.0 } else { 0.0 })],
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
            "mush_store_write_lane_depth",
            "gauge",
            "Accepted store work not yet completed, by independently bounded lane.",
            &[
                (&[("lane", "legacy")], self.store_legacy_queue_depth as f64),
                (
                    &[("lane", "authoritative")],
                    self.store_authoritative_queue_depth as f64,
                ),
                (
                    &[("lane", "control")],
                    self.store_control_queue_depth as f64,
                ),
            ],
        );
        metric(
            &mut o,
            "mush_store_last_ack_unixtime",
            "gauge",
            "Unix time of the last acknowledged store write (0 = none yet).",
            &[(&[], self.store_last_ack_unix as f64)],
        );

        // IPC boundary bounding (M18). The high-water gauge is labeled by
        // direction — never per-player or secret (mirrors the store block).
        metric(
            &mut o,
            "mush_ipc_frames_shed_total",
            "counter",
            "Input frames shed gateway->server when the to_server queue was full.",
            &[(&[], self.ipc_frames_shed_total as f64)],
        );
        metric(
            &mut o,
            "mush_ipc_write_timeouts_total",
            "counter",
            "IPC write_frame calls that timed out on a wedged peer (either direction).",
            &[(&[], self.ipc_write_timeouts_total as f64)],
        );
        metric(
            &mut o,
            "mush_ipc_to_gw_overflow_total",
            "counter",
            "Server->gateway (to_gw) overflow teardowns: a must-deliver frame hit a full queue.",
            &[(&[], self.ipc_to_gw_overflow_total as f64)],
        );
        metric(
            &mut o,
            "mush_ipc_reannounce_protocol_violations_total",
            "counter",
            "Oversized ReAnnounce snapshots rejected atomically before boundary teardown.",
            &[(&[], self.ipc_reannounce_protocol_violations_total as f64)],
        );
        metric(
            &mut o,
            "mush_ipc_retained_claims_reaped_total",
            "counter",
            "Retained coordinator claims freed by the server's retention reaper.",
            &[(&[], self.ipc_retained_claims_reaped_total as f64)],
        );
        metric(
            &mut o,
            "mush_coordinator_preflight_deferred_total",
            "counter",
            "Coordinator submissions deferred because preflight stayed transient to its bound.",
            &[(&[], self.coordinator_preflight_deferred_total as f64)],
        );
        metric(
            &mut o,
            "mush_ipc_queue_high_water",
            "gauge",
            "High-water mark of an IPC boundary queue's depth, by direction.",
            &[
                (
                    &[("dir", "to_server")],
                    self.ipc_queue_high_water_to_server as f64,
                ),
                (&[("dir", "to_gw")], self.ipc_queue_high_water_to_gw as f64),
            ],
        );

        // Operational custody behind one unresolved exact receipt. Barrier and
        // reattach dimensions are hard-bounded; sends/effects expose the legacy
        // pre-K2 construction gap without pretending that shedding is safe.
        metric(
            &mut o,
            "mush_commit_pending_barrier_refused_total",
            "counter",
            "Barrier waiters refused at the fixed pending-receipt cap.",
            &[(&[], self.commit_pending_barrier_refused_total as f64)],
        );
        metric(
            &mut o,
            "mush_commit_pending_reattach_coalesced_total",
            "counter",
            "Older full reattach snapshots replaced while an exact receipt was pending.",
            &[(&[], self.commit_pending_reattach_coalesced_total as f64)],
        );
        metric(
            &mut o,
            "mush_commit_pending_reattach_rejected_total",
            "counter",
            "Reattach snapshots rejected atomically for violating lifecycle custody.",
            &[(&[], self.commit_pending_reattach_rejected_total as f64)],
        );
        metric(
            &mut o,
            "mush_commit_pending_custody_items",
            "gauge",
            "Items held behind one unresolved exact receipt, by kind.",
            &[
                (&[("kind", "barrier")], self.commit_pending_barriers as f64),
                (
                    &[("kind", "reattach")],
                    self.commit_pending_reattach_records as f64,
                ),
                (&[("kind", "send")], self.commit_pending_sends as f64),
                (&[("kind", "effect")], self.commit_pending_effects as f64),
            ],
        );
        metric(
            &mut o,
            "mush_commit_pending_operational_custody_bytes",
            "gauge",
            "Conservative logical bytes of hard-bounded barrier and reattach custody.",
            &[(&[], self.commit_pending_operational_bytes as f64)],
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
        metric(
            &mut o,
            "mush_npc_decisions",
            "gauge",
            "Durable NPC decisions by pending state (H2a).",
            &[
                (&[("state", "ready")], self.npc_decisions_ready as f64),
                (&[("state", "leased")], self.npc_decisions_leased as f64),
                (
                    &[("state", "resolving")],
                    self.npc_decisions_resolving as f64,
                ),
            ],
        );

        // The economy journal (J1, doc 12 D-12-1 / D-12-3).
        metric(
            &mut o,
            "mush_economy_journal_entries",
            "gauge",
            "Journal transactions the economy holds.",
            &[(&[], self.economy_journal_entries as f64)],
        );
        metric(
            &mut o,
            "mush_economy_journal_per_hour",
            "gauge",
            "Journal transactions posted per hour since the economy booted.",
            &[(&[], self.economy_journal_per_hour as f64)],
        );
        metric(
            &mut o,
            "mush_economy_replay_us_per_entry",
            "gauge",
            "The last cold journal replay's cost per entry, microseconds.",
            &[(&[], self.economy_replay_us_per_entry as f64)],
        );
        metric(
            &mut o,
            "mush_economy_replay_ms",
            "gauge",
            "The last cold journal replay, milliseconds.",
            &[(&[], self.economy_replay_ms as f64)],
        );
        metric(
            &mut o,
            "mush_economy_fetch_ms",
            "gauge",
            "The last boot's economy namespace fetch, milliseconds.",
            &[(&[], self.economy_fetch_ms as f64)],
        );

        // The ledger archive tier (J5, doc 12 section 10).
        metric(
            &mut o,
            "mush_ledger_archive_chunks",
            "gauge",
            "Sealed ledger chunks the archive tier holds.",
            &[(&[], self.ledger_archive_chunks as f64)],
        );
        metric(
            &mut o,
            "mush_ledger_archive_purged_chunks",
            "gauge",
            "Archived chunks the primary store has been purged of.",
            &[(&[], self.ledger_archive_purged_chunks as f64)],
        );
        metric(
            &mut o,
            "mush_ledger_resident_bytes",
            "gauge",
            "Ledger bytes resident in the primary store after the last archive pass (-1 before one).",
            &[(&[], self.ledger_resident_bytes as f64)],
        );
        metric(
            &mut o,
            "mush_ledger_archive_last_pass_unixtime",
            "gauge",
            "When the last archive pass ran (0 before one).",
            &[(&[], self.ledger_archive_last_pass_unix as f64)],
        );
        metric(
            &mut o,
            "mush_ledger_archive_last_pass_ok",
            "gauge",
            "Whether the last archive pass finished without a warning.",
            &[(&[], self.ledger_archive_last_pass_ok as f64)],
        );

        // LLM aggregates (keyed only by feature route + provider — never
        // per-player, never a key or prompt).
        self.write_llm(&mut o);

        o
    }

    fn write_lag_histogram(&self, o: &mut String) {
        write_histogram(
            o,
            "mush_loop_lag_ms",
            "World-loop iteration lag histogram, milliseconds.",
            &LAG_BUCKETS_MS,
            &self.lag_buckets,
            self.lag_count,
            self.lag_sum_ms,
        );
    }

    /// The authoritative lane (H3): three histograms, the outcome counters,
    /// and the slot gauges. Every number is an aggregate.
    fn write_authoritative_lane(&self, o: &mut String) {
        write_histogram(
            o,
            "mush_authoritative_slot_hold_ms",
            "How long the single authoritative pending-intent slot was held per commit, milliseconds.",
            &LAG_BUCKETS_MS,
            &self.auth_hold_buckets,
            self.auth_hold_count,
            self.auth_hold_sum_ms,
        );
        write_histogram(
            o,
            "mush_authoritative_bundle_bytes",
            "Canonical encoded size of each authoritative bundle, bytes.",
            &BUNDLE_BUCKETS_BYTES,
            &self.auth_bytes_buckets,
            self.auth_bytes_count,
            self.auth_bytes_sum,
        );
        write_histogram(
            o,
            "mush_authoritative_replay_writes",
            "Replay writes carried by each authoritative bundle.",
            &WRITE_BUCKETS,
            &self.auth_writes_buckets,
            self.auth_writes_count,
            self.auth_writes_sum,
        );
        metric(
            o,
            "mush_authoritative_commits_total",
            "counter",
            "Settled authoritative commits since boot, by outcome.",
            &[
                (
                    &[("outcome", "finalized")],
                    self.auth_commits_finalized_total as f64,
                ),
                (
                    &[("outcome", "refused")],
                    self.auth_commits_refused_total as f64,
                ),
                (
                    &[("outcome", "integrity")],
                    self.auth_commits_integrity_total as f64,
                ),
                (
                    &[("outcome", "quarantined")],
                    self.auth_commits_quarantined_total as f64,
                ),
            ],
        );
        metric(
            o,
            "mush_authoritative_slot_held",
            "gauge",
            "1 while the single authoritative pending-intent slot is held, else 0.",
            &[(&[], if self.auth_slot_held { 1.0 } else { 0.0 })],
        );
        metric(
            o,
            "mush_authoritative_slot_held_since_unixtime",
            "gauge",
            "Unix time the authoritative slot was last taken (0 = never).",
            &[(&[], self.auth_slot_held_since_unix as f64)],
        );
        metric(
            o,
            "mush_authoritative_last_bundle_bytes",
            "gauge",
            "Canonical encoded size of the most recent authoritative bundle, bytes.",
            &[(&[], self.auth_bytes_last as f64)],
        );
        metric(
            o,
            "mush_authoritative_last_replay_writes",
            "gauge",
            "Replay writes carried by the most recent authoritative bundle.",
            &[(&[], self.auth_writes_last as f64)],
        );
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
/// The bucket upper bound in which the 99th-percentile observation of a
/// cumulative histogram falls; `max` when it lands in the open-topped `+Inf`
/// bucket; `0` when nothing was observed.
fn histogram_p99(bounds: &[u64], counts: &[u64], count: u64, max: u64) -> u64 {
    if count == 0 {
        return 0;
    }
    // rank of the p99 observation (1-based), ceil(0.99 * count).
    let target = count.saturating_mul(99).div_ceil(100).max(1);
    for (i, &bound) in bounds.iter().enumerate() {
        if counts[i] >= target {
            return bound;
        }
    }
    max
}

/// One Prometheus histogram family from cumulative bucket counts. Each
/// bucket already holds the count of observations `<= le` (the observer
/// increments every bucket whose bound the sample fits), which is exactly the
/// Prometheus bucket value.
fn write_histogram(
    o: &mut String,
    name: &str,
    help: &str,
    bounds: &[u64],
    counts: &[u64],
    count: u64,
    sum: u64,
) {
    o.push_str(&format!("# HELP {name} {help}\n"));
    o.push_str(&format!("# TYPE {name} histogram\n"));
    for (i, &bound) in bounds.iter().enumerate() {
        o.push_str(&format!("{name}_bucket{{le=\"{bound}\"}} {}\n", counts[i]));
    }
    o.push_str(&format!("{name}_bucket{{le=\"+Inf\"}} {count}\n"));
    o.push_str(&format!("{name}_sum {sum}\n"));
    o.push_str(&format!("{name}_count {count}\n"));
}

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

    // ---- H3: the authoritative lane ----

    #[test]
    fn h3_authoritative_commit_lands_in_every_bucket_at_or_above_its_value() {
        let m = Metrics::new();
        m.authoritative_slot_held(1_000);
        assert!(m.snapshot().auth_slot_held);
        assert_eq!(m.snapshot().auth_slot_held_since_unix, 1_000);
        m.authoritative_slot_released();
        m.observe_authoritative_commit(Some(7), 5_000, 3, AuthoritativeCommitOutcome::Finalized);
        let s = m.snapshot();
        assert!(!s.auth_slot_held);
        assert_eq!(s.auth_hold_last_ms, 7);
        assert_eq!(s.auth_hold_max_ms, 7);
        assert_eq!(s.auth_hold_count, 1);
        assert_eq!(s.auth_hold_sum_ms, 7);
        // 7 ms fits every bound from 10 up, not 1/2/5.
        for (i, &bound) in LAG_BUCKETS_MS.iter().enumerate() {
            assert_eq!(s.auth_hold_buckets[i], u64::from(bound >= 7), "le={bound}");
        }
        // 5 000 bytes fits 16 KiB and up, not 1 KiB / 4 KiB.
        for (i, &bound) in BUNDLE_BUCKETS_BYTES.iter().enumerate() {
            assert_eq!(
                s.auth_bytes_buckets[i],
                u64::from(bound >= 5_000),
                "le={bound}"
            );
        }
        // 3 writes fits 4 and up.
        for (i, &bound) in WRITE_BUCKETS.iter().enumerate() {
            assert_eq!(
                s.auth_writes_buckets[i],
                u64::from(bound >= 3),
                "le={bound}"
            );
        }
        assert_eq!(s.auth_commits_finalized_total, 1);
        assert_eq!(s.auth_commits_refused_total, 0);
        assert_eq!(s.auth_bytes_last, 5_000);
        assert_eq!(s.auth_writes_last, 3);
    }

    #[test]
    fn h3_authoritative_p99_and_mean_follow_the_loop_lag_rule() {
        let m = Metrics::new();
        assert_eq!(m.snapshot().authoritative_hold_p99_ms(), 0);
        assert_eq!(m.snapshot().authoritative_hold_mean_ms(), 0);
        for _ in 0..99 {
            m.observe_authoritative_commit(Some(1), 100, 1, AuthoritativeCommitOutcome::Finalized);
        }
        m.observe_authoritative_commit(Some(900), 100, 1, AuthoritativeCommitOutcome::Integrity);
        m.authoritative_bundle_quarantined();
        let s = m.snapshot();
        // 99 fast and 1 slow: the p99 rank is still the fast bucket.
        assert_eq!(s.authoritative_hold_p99_ms(), 1);
        assert_eq!(s.authoritative_hold_mean_ms(), 9);
        assert_eq!(s.auth_commits_integrity_total, 1);
        assert_eq!(s.auth_commits_quarantined_total, 1);
        // A refusal is counted and sized but never enters the hold histogram.
        m.observe_authoritative_commit(None, 100, 1, AuthoritativeCommitOutcome::Refused);
        let s = m.snapshot();
        assert_eq!(s.auth_commits_refused_total, 1);
        assert_eq!(s.auth_hold_count, 100);
        assert_eq!(s.auth_bytes_count, 101);
        assert_eq!(s.auth_writes_count, 101);
    }

    #[test]
    fn h3_authoritative_lane_renders_and_carries_only_aggregates() {
        let m = Metrics::new();
        m.observe_authoritative_commit(Some(12), 2_048, 2, AuthoritativeCommitOutcome::Finalized);
        m.authoritative_slot_held(77);
        let text = m.snapshot().to_prometheus(1);
        for needle in [
            "# TYPE mush_authoritative_slot_hold_ms histogram",
            "mush_authoritative_slot_hold_ms_bucket{le=\"25\"} 1",
            "mush_authoritative_slot_hold_ms_bucket{le=\"+Inf\"} 1",
            "mush_authoritative_slot_hold_ms_sum 12",
            "# TYPE mush_authoritative_bundle_bytes histogram",
            "mush_authoritative_bundle_bytes_bucket{le=\"4096\"} 1",
            "mush_authoritative_bundle_bytes_bucket{le=\"1024\"} 0",
            "# TYPE mush_authoritative_replay_writes histogram",
            "mush_authoritative_replay_writes_bucket{le=\"2\"} 1",
            "mush_authoritative_commits_total{outcome=\"finalized\"} 1",
            "mush_authoritative_commits_total{outcome=\"quarantined\"} 0",
            "mush_authoritative_slot_held 1",
            "mush_authoritative_slot_held_since_unixtime 77",
            "mush_authoritative_last_bundle_bytes 2048",
            "mush_authoritative_last_replay_writes 2",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
    }

    /// H2a: the decision census renders as one gauge family by state, and
    /// `@stats`'s numbers come from the same snapshot.
    #[test]
    fn h2a_npc_decisions_gauge_renders_by_state() {
        let m = Metrics::new();
        m.publish_world(WorldSample {
            sessions_current: 0,
            connections_current: 0,
            scheduler_queue_depth: 0,
            rooms: 1,
            exits: 0,
            things: 0,
            players: 1,
            npcs: 1,
            store_read_only: false,
            integrity_frozen: false,
            npc_active: 1,
            npc_thinking: 3,
            npc_decisions_ready: 1,
            npc_decisions_leased: 2,
            npc_decisions_resolving: 0,
            ..WorldSample::default()
        });
        let s = m.snapshot();
        assert_eq!(
            (
                s.npc_decisions_ready,
                s.npc_decisions_leased,
                s.npc_decisions_resolving
            ),
            (1, 2, 0)
        );
        let text = s.to_prometheus(1);
        for needle in [
            "# TYPE mush_npc_decisions gauge",
            "mush_npc_decisions{state=\"ready\"} 1",
            "mush_npc_decisions{state=\"leased\"} 2",
            "mush_npc_decisions{state=\"resolving\"} 0",
            "mush_npc_thinking 3",
        ] {
            assert!(
                text.contains(needle),
                "missing {needle:?} in:
{text}"
            );
        }
    }

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
    fn ipc_bounding_counters_and_high_water() {
        let m = Metrics::new();
        // Fresh registry: all IPC bounding metrics zero.
        let s = m.snapshot();
        assert_eq!(s.ipc_frames_shed_total, 0);
        assert_eq!(s.ipc_write_timeouts_total, 0);
        assert_eq!(s.ipc_to_gw_overflow_total, 0);
        assert_eq!(s.ipc_reannounce_protocol_violations_total, 0);
        assert_eq!(s.ipc_retained_claims_reaped_total, 0);
        assert_eq!(s.coordinator_preflight_deferred_total, 0);
        assert_eq!(s.ipc_queue_high_water_to_server, 0);
        assert_eq!(s.ipc_queue_high_water_to_gw, 0);

        // Counters are monotonic.
        for _ in 0..3 {
            m.ipc_frame_shed();
        }
        m.ipc_write_timeout();
        m.ipc_write_timeout();
        m.ipc_to_gw_overflow();
        m.ipc_reannounce_protocol_violation();
        // A reaper pass frees a whole batch at once, so this counter takes the
        // pass's count rather than one bump per claim.
        m.ipc_retained_claims_reaped(2);
        m.ipc_retained_claims_reaped(3);
        // Deferral is per submission: one bump per handler that gave up
        // at the bound, in either deployment arm.
        m.coordinator_preflight_deferred();

        // High-water keeps the max, not the last observation.
        m.observe_ipc_to_server_depth(5);
        m.observe_ipc_to_server_depth(12);
        m.observe_ipc_to_server_depth(7);
        m.observe_ipc_to_gw_depth(4000);
        m.observe_ipc_to_gw_depth(1000);

        let s = m.snapshot();
        assert_eq!(s.ipc_frames_shed_total, 3);
        assert_eq!(m.ipc_frames_shed_total(), 3);
        assert_eq!(s.ipc_write_timeouts_total, 2);
        assert_eq!(s.ipc_to_gw_overflow_total, 1);
        assert_eq!(s.ipc_reannounce_protocol_violations_total, 1);
        assert_eq!(s.ipc_retained_claims_reaped_total, 5);
        assert_eq!(s.coordinator_preflight_deferred_total, 1);
        assert_eq!(s.ipc_queue_high_water_to_server, 12);
        assert_eq!(s.ipc_queue_high_water_to_gw, 4000);

        // Prometheus renders them, with the high-water labeled by direction
        // and no secret label.
        let text = s.to_prometheus(1);
        assert!(text.contains("# TYPE mush_ipc_frames_shed_total counter"));
        assert!(text.contains("mush_ipc_frames_shed_total 3"));
        assert!(text.contains("mush_ipc_write_timeouts_total 2"));
        assert!(text.contains("mush_ipc_to_gw_overflow_total 1"));
        assert!(text.contains("mush_ipc_reannounce_protocol_violations_total 1"));
        assert!(text.contains("mush_ipc_retained_claims_reaped_total 5"));
        assert!(text.contains("mush_coordinator_preflight_deferred_total 1"));
        assert!(text.contains("# TYPE mush_ipc_queue_high_water gauge"));
        assert!(text.contains("mush_ipc_queue_high_water{dir=\"to_server\"} 12"));
        assert!(text.contains("mush_ipc_queue_high_water{dir=\"to_gw\"} 4000"));
        for forbidden in ["player=\"", "account=\"", "token=\"", "ip=\"", "actor=\""] {
            assert!(!text.contains(forbidden));
        }
    }

    #[test]
    fn commit_pending_custody_counters_and_gauges_render() {
        let m = Metrics::new();
        m.commit_pending_barrier_refused();
        m.commit_pending_reattach_coalesced();
        m.commit_pending_reattach_coalesced();
        m.commit_pending_reattach_rejected();
        m.set_commit_pending_custody(256, 1, 65_600, 3, 2);

        let s = m.snapshot();
        assert_eq!(s.commit_pending_barrier_refused_total, 1);
        assert_eq!(s.commit_pending_reattach_coalesced_total, 2);
        assert_eq!(s.commit_pending_reattach_rejected_total, 1);
        assert_eq!(s.commit_pending_barriers, 256);
        assert_eq!(s.commit_pending_reattach_records, 1);
        assert_eq!(s.commit_pending_operational_bytes, 65_600);
        assert_eq!(s.commit_pending_sends, 3);
        assert_eq!(s.commit_pending_effects, 2);

        let text = s.to_prometheus(1);
        assert!(text.contains("mush_commit_pending_barrier_refused_total 1"));
        assert!(text.contains("mush_commit_pending_reattach_coalesced_total 2"));
        assert!(text.contains("mush_commit_pending_reattach_rejected_total 1"));
        assert!(text.contains("mush_commit_pending_custody_items{kind=\"barrier\"} 256"));
        assert!(text.contains("mush_commit_pending_custody_items{kind=\"reattach\"} 1"));
        assert!(text.contains("mush_commit_pending_custody_items{kind=\"send\"} 3"));
        assert!(text.contains("mush_commit_pending_custody_items{kind=\"effect\"} 2"));
        assert!(text.contains("mush_commit_pending_operational_custody_bytes 65600"));
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
        assert_eq!(j["integrity_ok"], true);
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
        m.add_store_write_queue_depth(6);
        m.add_store_legacy_queue_depth(3);
        m.add_store_authoritative_queue_depth(2);
        m.add_store_control_queue_depth(1);
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
            integrity_frozen: false,
            npc_active: 1,
            npc_thinking: 0,
            npc_decisions_ready: 0,
            npc_decisions_leased: 0,
            npc_decisions_resolving: 0,
            ..WorldSample::default()
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
        assert!(text.contains("mush_integrity_frozen 0"));
        assert!(text.contains("mush_store_write_queue_depth 6"));
        assert!(text.contains("mush_store_write_lane_depth{lane=\"legacy\"} 3"));
        assert!(text.contains("mush_store_write_lane_depth{lane=\"authoritative\"} 2"));
        assert!(text.contains("mush_store_write_lane_depth{lane=\"control\"} 1"));
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

    /// J1 (doc 12 D-12-1 / D-12-3): the economy journal gauges render.
    #[test]
    fn economy_journal_gauges_render() {
        let m = Metrics::new();
        m.publish_world(WorldSample {
            economy_journal_entries: 100_000,
            economy_journal_per_hour: 15_000,
            economy_replay_us_per_entry: 22,
            economy_replay_ms: 2_200,
            economy_fetch_ms: 480,
            ..WorldSample::default()
        });
        let s = m.snapshot();
        assert_eq!(s.economy_journal_entries, 100_000);
        let text = s.to_prometheus(1);
        for needle in [
            "# TYPE mush_economy_journal_entries gauge",
            "mush_economy_journal_entries 100000",
            "mush_economy_journal_per_hour 15000",
            "mush_economy_replay_us_per_entry 22",
            "mush_economy_replay_ms 2200",
            "mush_economy_fetch_ms 480",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
    }

    /// J5 (doc 12 section 10): the ledger archive gauges render.
    #[test]
    fn ledger_archive_gauges_render() {
        let m = Metrics::new();
        m.publish_ledger_archive(LedgerArchiveSample {
            chunks: 3_560,
            purged_chunks: 3_000,
            resident_bytes: 9_000_000,
            last_pass_unix: 1_700_000_000,
            last_pass_ok: true,
        });
        let text = m.snapshot().to_prometheus(1);
        for needle in [
            "mush_ledger_archive_chunks 3560",
            "mush_ledger_archive_purged_chunks 3000",
            "mush_ledger_resident_bytes 9000000",
            "mush_ledger_archive_last_pass_unixtime 1700000000",
            "mush_ledger_archive_last_pass_ok 1",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
    }
}
