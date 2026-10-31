//! Local and sequential store-backed staff operations, `@ps`/`@stats` rendering, and metrics sampling.

use super::*;

/// Handle the **local** staff ops (`@boot`, `@ps`, `@stats`) inline in the
/// world loop: they need the `Engine` (scheduler, world, loop-lag mirror)
/// and the session manager, and never touch the store. Output rides the
/// normal outbox; the caller delivers it right after.
pub(super) fn admit_staff_ops(
    mode: WorldMode,
    ops: Vec<StaffOp>,
    gateway: &Gateway,
) -> Vec<StaffOp> {
    let mut admitted = Vec::with_capacity(ops.len());
    for op in ops {
        match admission_for(mode, classify_staff_op(&op.kind)) {
            Admission::Allow => admitted.push(op),
            Admission::Refuse(reason) => {
                tracing::warn!(actor = %op.actor, ?reason, "staff operation suppressed by world mode");
                send_sys(gateway, op.actor, reason.text());
            }
            Admission::CheckStoreReadOnlyCommand => {
                // Staff ops are typed, so they never classify as a command
                // line and never need the read-only allowlist. An unresolved
                // admission is suppressed: not running the op is the fail-safe
                // half of a staff mutation.
                tracing::error!(
                    actor = %op.actor,
                    "typed staff operation hit command-text admission; suppressed"
                );
            }
        }
    }
    admitted
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_local_staff_ops(
    engine: &mut Engine,
    gateway: &Gateway,
    health: &StoreHealth,
    boot_unix: i64,
    ops: Vec<StaffOp>,
    out: &mut Outbox,
    fx: &mut Effects,
    llm: Option<&Arc<LlmService>>,
    metrics: &Metrics,
) {
    for op in ops {
        let actor = op.actor;
        match op.kind {
            StaffOpKind::Boot {
                target,
                ref message,
            } => {
                tracing::info!(%actor, %target, message, "@boot requested");
                let name = engine
                    .world
                    .get(target)
                    .map_or_else(|| target.to_string(), |o| o.name.clone());
                // Split mode (M16b): the forked server holds no `SessionManager`,
                // so it cannot walk `char`'s connections. Emit a fire-and-forget
                // `ToGateway::CloseSession` — the gateway resolves the char to
                // its live connections, drops them, and reports each resulting
                // `ConnDown(Boot)` back as inbound, which the world then steps
                // exactly as the in-process loop below does. The world loop
                // never blocks on a reply (D4). The target's Disconnect message
                // was already delivered as a normal `Deliver` frame ahead of
                // this, so socket FIFO ordering preserves "message then close".
                if gateway.is_socket() {
                    gateway.close_session(u64::from(target.0), mush_ipc::DetachCause::Booted);
                    out.info(actor, format!("@boot: {name} booted."));
                    continue;
                }
                // Close the whole session: every connection handle comes
                // back so each ConnDown(Boot) can be fed to the engine
                // (announce + world-side disconnect on the last one).
                // The handler's Disconnect event was already delivered, so
                // the target saw the message; dropping the handles closes
                // the transports once their queues drain.
                //
                // M16b (unrouted boundary crossing): `session_of_char` +
                // `close_session` read and mutate gateway-owned session state.
                // This becomes a `ToGateway::CloseSession{char,
                // DetachCause::Booted}`; the gateway closes the transports and
                // reports each resulting `ConnDown(Boot)` back as a
                // `ToServer::Inbound`, which the world steps exactly as the
                // loop below does. Kept direct in M16a (shared manager →
                // behaviour-identical).
                let mut booted = 0u32;
                for conn in gateway.close_char_connections(u64::from(target.0)) {
                    booted += 1;
                    engine.step(
                        Inbound::ConnDown {
                            conn,
                            actor: target,
                            reason: DisconnectKind::Boot,
                        },
                        out,
                        fx,
                    );
                }
                if booted == 0 {
                    // No live connections (linkdead, or a test world with
                    // no session layer attachment): force the world-side
                    // disconnect so WHO and the room agree.
                    engine.step(Inbound::SessionGone { actor: target }, out, fx);
                    out.info(
                        actor,
                        format!("@boot: {name} had no live connections; session cleared."),
                    );
                } else {
                    let plural = if booted == 1 {
                        "connection"
                    } else {
                        "connections"
                    };
                    out.info(actor, format!("@boot: {name} booted ({booted} {plural})."));
                }
            }
            StaffOpKind::Ps => {
                tracing::info!(%actor, "@ps requested");
                out.info(actor, render_ps(engine, unix_now_ms()));
            }
            StaffOpKind::Stats => {
                tracing::info!(%actor, "@stats requested");
                let extra = stats_extra(metrics, llm);
                out.info(
                    actor,
                    render_stats(engine, health, boot_unix, unix_now_s(), &extra),
                );
            }
            // Store ops never reach here (see the partition in `run`).
            _ => {}
        }
    }
}

/// Truncate a command line for `@ps` display.
pub(super) fn preview(line: &str) -> String {
    const MAX: usize = 40;
    if line.chars().count() <= MAX {
        line.to_string()
    } else {
        let cut: String = line.chars().take(MAX).collect();
        format!("{cut}...")
    }
}

/// Human form of a tick cadence: `coarse 300s`, `fast 250ms`, `scene`.
pub(super) fn cadence_text(c: TickCadence) -> String {
    match c {
        TickCadence::Coarse(d) => format!("coarse {}s", d.as_secs().max(1)),
        TickCadence::Fast(d) => format!("fast {}ms", d.as_millis().max(1)),
        TickCadence::Scene => "scene".to_string(),
    }
}

/// Render the `@ps` scheduler report: pending `@wait`s (actor, seconds
/// remaining, command preview), module ticks (tag, cadence, next fire),
/// scene ticks, and totals. Pure over the engine — unit-testable.
#[must_use]
pub fn render_ps(engine: &Engine, now_ms: i64) -> String {
    let snapshot = engine.sched.pending_entries();
    let entries = snapshot.entries();
    let mut next_fire: std::collections::HashMap<(String, String), i64> =
        std::collections::HashMap::new();
    let mut waits = Vec::new();
    let mut scenes = Vec::new();
    let mut control = 0usize;
    for &(due, kind) in entries {
        let remain_s = (due - now_ms).max(0) / 1000;
        match kind {
            ScheduledKind::WaitCmd { actor, line } => {
                let name = engine
                    .world
                    .get(*actor)
                    .map_or_else(|| actor.to_string(), |o| o.name.clone());
                waits.push(format!("  {name}  in {remain_s}s: {}", preview(line)));
            }
            ScheduledKind::ModuleTick { module, tag } => {
                next_fire.insert((module.to_string(), tag.to_string()), remain_s);
            }
            ScheduledKind::SceneTick { module, tag, scene } => {
                scenes.push(format!("  {module}:{tag}  {scene}  next in {remain_s}s"));
            }
            ScheduledKind::Control(_) => control += 1,
            // Internal deadlines and registered continuations are ephemeral;
            // their private request arguments are not shown in @ps.
            ScheduledKind::RequestTimeout { .. }
            | ScheduledKind::CommandContinuation(_)
            | ScheduledKind::ConnectionCompletion(_) => {}
        }
    }
    let mut ticks = Vec::new();
    for t in engine.registry.module_ticks() {
        let when = next_fire
            .get(&(t.module.to_string(), t.tag.to_string()))
            .map_or_else(String::new, |s| format!("  next in {s}s"));
        ticks.push(format!(
            "  {}:{}  {}{when}",
            t.module,
            t.tag,
            cadence_text(t.cadence)
        ));
    }
    let mut text = String::from("Scheduler:");
    text.push_str(&format!("\nWaits: {}", waits.len()));
    for w in &waits {
        text.push('\n');
        text.push_str(w);
    }
    text.push_str(&format!("\nModule ticks: {}", ticks.len()));
    for t in &ticks {
        text.push('\n');
        text.push_str(t);
    }
    text.push_str(&format!("\nScene ticks: {}", scenes.len()));
    for s in &scenes {
        text.push('\n');
        text.push_str(s);
    }
    if control > 0 {
        text.push_str(&format!("\nControl timers: {control}"));
    }
    text.push_str(&format!("\nTotal scheduled entries: {}", snapshot.total()));
    text.push_str(&format!(
        "\nDisplayed earliest entries: {} ({} omitted)",
        entries.len(),
        snapshot.omitted()
    ));
    text
}

/// Object counts by kind, plus a count of husks pending destruction.
pub(super) struct ObjectCounts {
    pub(super) rooms: usize,
    pub(super) exits: usize,
    pub(super) things: usize,
    pub(super) players: usize,
    pub(super) npcs: usize,
    pub(super) going: usize,
}

impl ObjectCounts {
    pub(super) fn total(&self) -> usize {
        self.rooms + self.exits + self.things + self.players + self.npcs
    }
}

/// Scan the world once for object counts by kind (GOING husks excluded from
/// the kind tallies, counted separately). Shared by `@stats` and the metrics
/// sample.
pub(super) fn object_counts(w: &World) -> ObjectCounts {
    let mut c = ObjectCounts {
        rooms: 0,
        exits: 0,
        things: 0,
        players: 0,
        npcs: 0,
        going: 0,
    };
    for o in w.iter() {
        if o.flags.contains(Flags::GOING) {
            c.going += 1;
            continue;
        }
        match o.kind {
            ObjKind::Room => c.rooms += 1,
            ObjKind::Exit => c.exits += 1,
            ObjKind::Thing => c.things += 1,
            ObjKind::Player => c.players += 1,
            ObjKind::Npc => c.npcs += 1,
        }
    }
    c
}

/// NPC agent runtime counts read from the module state slot: `active` = NPCs
/// with a live runtime (stimulated at least once this boot), `thinking` = NPCs
/// with an in-flight LLM turn. `(0, 0)` when the NPC module is not loaded.
/// The NPC indicator (H2a): `active` is every NPC with a live runtime;
/// `thinking` is every dialogue turn in flight **plus** every non-terminal
/// durable decision (either kind of thinking); the census splits the
/// decisions by state for `mush_npc_decisions{state}`.
pub(super) fn npc_counts(engine: &Engine) -> (i64, i64, mush_core::DecisionStatusCensus) {
    let (active, in_flight) = match engine.world.module_state::<mush_npc::NpcHostState>() {
        Some(h) => (
            h.runtimes.len() as i64,
            h.runtimes.values().filter(|rt| rt.in_flight).count() as i64,
        ),
        None => (0, 0),
    };
    let census = decision_census(engine);
    let thinking =
        in_flight.saturating_add(i64::try_from(census.nonterminal()).unwrap_or(i64::MAX));
    (active, thinking, census)
}

/// J1 (doc 12 D-12-1, D-12-3): the economy journal's size, its growth since
/// boot, and what the last cold replay cost — the numbers the checkpoint
/// decision keys on. Zero throughout when the economy module is not composed.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct EconomyJournalSample {
    pub(super) entries: i64,
    pub(super) per_hour: i64,
    pub(super) replay_us_per_entry: i64,
    pub(super) replay_ms: i64,
    pub(super) fetch_ms: i64,
}

pub(super) fn economy_journal(engine: &Engine) -> EconomyJournalSample {
    let Some(store) = engine.world.module_state::<mush_economy::EconomyStore>() else {
        return EconomyJournalSample::default();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let uptime = now.saturating_sub(store.last_boot()).max(1);
    let posts = i64::try_from(store.posts_since_boot()).unwrap_or(i64::MAX);
    let per_hour = posts.saturating_mul(3_600) / uptime;
    let replay = store.boot_replay();
    let replay_ms = replay.map_or(0, |r| i64::try_from(r.elapsed_ms).unwrap_or(i64::MAX));
    let fetch_ms = replay.map_or(0, |r| i64::try_from(r.fetch_ms).unwrap_or(i64::MAX));
    let replay_us_per_entry = replay.filter(|r| r.entries > 0).map_or(0, |r| {
        let entries = u64::try_from(r.entries).unwrap_or(u64::MAX);
        i64::try_from(r.elapsed_ms.saturating_mul(1_000) / entries).unwrap_or(i64::MAX)
    });
    EconomyJournalSample {
        entries: i64::try_from(store.journal_len()).unwrap_or(i64::MAX),
        per_hour,
        replay_us_per_entry,
        replay_ms,
        fetch_ms,
    }
}

/// Every composed decision source's pending-state census, page by page. A
/// scan fault (a malformed record) reports zero rather than freezing a
/// metrics sample; the boot audit is what refuses such a record.
pub(super) fn decision_census(engine: &Engine) -> mush_core::DecisionStatusCensus {
    let mut census = mush_core::DecisionStatusCensus::default();
    if !engine
        .registry
        .decision_sources()
        .is_some_and(|sources| !sources.is_empty())
    {
        return census;
    }
    let mut cursor = None;
    loop {
        let Ok(page) = engine
            .discover_decision_dispatches(cursor.as_ref(), mush_core::MAX_DECISION_RECOVERY_PAGE)
        else {
            return mush_core::DecisionStatusCensus::default();
        };
        census.merge(page.status_census());
        cursor = page.next_cursor().cloned();
        if cursor.is_none() {
            return census;
        }
    }
}

/// The per-`(feature, provider)` cumulative LLM stats + today's global spend,
/// pulled from the service's audit aggregate and budget ledger. `None` when no
/// LLM service is wired.
pub(super) struct LlmSample {
    pub(super) globally_disabled: bool,
    pub(super) features: Vec<LlmFeatureStat>,
    pub(super) today: LlmTodaySpend,
}

pub(super) fn llm_sample(llm: Option<&Arc<LlmService>>) -> Option<LlmSample> {
    let llm = llm?;
    let features = llm
        .audit_log()
        .totals()
        .into_iter()
        .map(|t| LlmFeatureStat {
            feature: t.feature,
            provider: t.provider,
            calls: t.calls,
            refusals: t.refusals,
            budget_trips: t.budget_trips,
            disabled: t.disabled,
            errors: t.errors,
            input_tokens: t.input_tokens,
            output_tokens: t.output_tokens,
            cost_usd: t.cost_usd,
        })
        .collect();
    let g = llm.ledger().global();
    Some(LlmSample {
        globally_disabled: llm.kill_switch().global_disabled(),
        features,
        today: LlmTodaySpend {
            input_tokens: g.input_tokens,
            output_tokens: g.output_tokens,
            cost_usd: g.cost_usd,
        },
    })
}

/// Publish the world-owned gauges into the metrics registry, throttled to at
/// most once a second so the object scan + NPC/LLM aggregation never rides the
/// hot path. Called at the end of each world-loop iteration.
pub(super) fn maybe_publish_sample(
    engine: &Engine,
    gateway: &Gateway,
    health: &StoreHealth,
    llm: Option<&Arc<LlmService>>,
    metrics: &Metrics,
    last_sample: &mut Option<Instant>,
) {
    const MIN_INTERVAL: Duration = Duration::from_secs(1);
    let now = Instant::now();
    if let Some(prev) = last_sample
        && now.duration_since(*prev) < MIN_INTERVAL
    {
        return;
    }
    *last_sample = Some(now);

    let counts = object_counts(&engine.world);
    let (npc_active, npc_thinking, decisions) = npc_counts(engine);
    let economy = economy_journal(engine);
    // The session/connection gauges are gateway-owned: the in-process sink reads
    // the shared manager, the socket sink reports `(0, 0)` (the gateway process
    // publishes them post-fork). Advisory gauges, not world state.
    let (sessions_current, connections_current) = gateway.session_counts();
    metrics.publish_world(WorldSample {
        sessions_current,
        connections_current,
        scheduler_queue_depth: engine.sched.len() as i64,
        rooms: counts.rooms as i64,
        exits: counts.exits as i64,
        things: counts.things as i64,
        players: counts.players as i64,
        npcs: counts.npcs as i64,
        store_read_only: health.mode.store_read_only(),
        integrity_frozen: health.mode.integrity_frozen(),
        npc_active,
        npc_thinking,
        npc_decisions_ready: i64::try_from(decisions.ready).unwrap_or(i64::MAX),
        npc_decisions_leased: i64::try_from(decisions.leased).unwrap_or(i64::MAX),
        npc_decisions_resolving: i64::try_from(decisions.resolving + decisions.fallback_ready)
            .unwrap_or(i64::MAX),
        economy_journal_entries: economy.entries,
        economy_journal_per_hour: economy.per_hour,
        economy_replay_us_per_entry: economy.replay_us_per_entry,
        economy_replay_ms: economy.replay_ms,
        economy_fetch_ms: economy.fetch_ms,
    });
    if let Some(s) = llm_sample(llm) {
        metrics.publish_llm(true, s.globally_disabled, s.features, s.today);
    } else {
        metrics.publish_llm(false, false, Vec::new(), LlmTodaySpend::default());
    }
}

/// Extra aggregates threaded into the wizard `@stats` render beyond what the
/// engine + store health carry: transport-split connection counts, the store
/// write-queue depth, NPC runtime counts, and today's LLM spend.
#[derive(Clone, Debug, Default)]
pub struct StatsExtra {
    /// Live SSH transport connections.
    pub ssh_conns: i64,
    /// Live web transport connections.
    pub web_conns: i64,
    /// Store write batches queued but not yet acked.
    pub write_queue_depth: i64,
    /// NPCs with a live runtime.
    pub npc_active: i64,
    /// NPCs with an in-flight LLM turn.
    pub npc_thinking: i64,
    /// H2a: durable NPC decisions by state — ready, leased, resolving.
    pub npc_decisions: [i64; 3],
    /// Approximate p99 loop lag (ms) from the histogram.
    pub lag_p99: u64,
    /// The authoritative lane (H3): whether the single pending-intent slot is
    /// held right now.
    pub auth_slot_held: bool,
    /// Most recent authoritative slot hold, ms.
    pub auth_hold_last_ms: u64,
    /// Approximate p99 authoritative slot hold, ms.
    pub auth_hold_p99_ms: u64,
    /// Longest authoritative slot hold since boot, ms.
    pub auth_hold_max_ms: u64,
    /// Most recent authoritative bundle's canonical encoded size, bytes.
    pub auth_last_bundle_bytes: u64,
    /// Most recent authoritative bundle's replay-write count.
    pub auth_last_replay_writes: u64,
    /// Settled authoritative commits since boot: finalized, refused,
    /// integrity, quarantined.
    pub auth_commits: [u64; 4],
    /// Input frames shed gateway->server since boot (M18 IPC bounding).
    pub ipc_frames_shed: u64,
    /// IPC write timeouts on a wedged peer since boot (M18).
    pub ipc_write_timeouts: u64,
    /// Server->gateway (`to_gw`) overflow teardowns since boot (M18).
    pub ipc_to_gw_overflow: u64,
    /// High-water mark of the gateway->server (`to_server`) queue depth (M18).
    pub ipc_hw_to_server: u64,
    /// High-water mark of the server->gateway (`to_gw`) queue depth (M18).
    pub ipc_hw_to_gw: u64,
    /// Gateway links open right now (doc 33 §4.3; 0 in the all-in-one).
    pub ipc_links: u64,
    /// Today's LLM spend, when a service is wired.
    pub llm: Option<LlmStatsLine>,
    /// J1 (doc 12 D-12-1): the economy journal — entries held, posted per hour
    /// since boot, the last replay's µs per entry, the last replay ms, the
    /// last fetch ms.
    pub economy_journal: [i64; 5],
}

/// Today's LLM spend for the `@stats` LLM line.
#[derive(Clone, Debug)]
pub struct LlmStatsLine {
    /// Whether the global kill switch is engaged.
    pub globally_disabled: bool,
    /// Today's global input tokens.
    pub today_input: u64,
    /// Today's global output tokens.
    pub today_output: u64,
    /// Today's global cost, USD.
    pub today_cost: f64,
    /// Per-feature today spend `(feature, input, output, cost)`, only rows
    /// with non-zero usage.
    pub features: Vec<(String, u64, u64, f64)>,
}

/// Build the [`StatsExtra`] for a `@stats` render from the live registries.
pub(super) fn stats_extra(metrics: &Metrics, llm: Option<&Arc<LlmService>>) -> StatsExtra {
    let snap = metrics.snapshot();
    let llm_line = llm.map(|svc| {
        let ledger = svc.ledger();
        let g = ledger.global();
        // Per-feature today spend: probe each route the audit aggregate has
        // seen (cheap — a handful) against today's ledger.
        let mut features = Vec::new();
        for t in svc.audit_log().totals() {
            let c = ledger.feature(&t.feature);
            if c.input_tokens + c.output_tokens > 0 || c.cost_usd > 0.0 {
                features.push((t.feature, c.input_tokens, c.output_tokens, c.cost_usd));
            }
        }
        features.sort_by(|a, b| a.0.cmp(&b.0));
        features.dedup_by(|a, b| a.0 == b.0);
        LlmStatsLine {
            globally_disabled: svc.kill_switch().global_disabled(),
            today_input: g.input_tokens,
            today_output: g.output_tokens,
            today_cost: g.cost_usd,
            features,
        }
    });
    StatsExtra {
        ssh_conns: snap.ssh_current,
        web_conns: snap.web_current,
        write_queue_depth: snap.store_write_queue_depth,
        npc_active: snap.npc_active,
        npc_thinking: snap.npc_thinking,
        npc_decisions: [
            snap.npc_decisions_ready,
            snap.npc_decisions_leased,
            snap.npc_decisions_resolving,
        ],
        lag_p99: snap.loop_lag_p99_ms(),
        auth_slot_held: snap.auth_slot_held,
        auth_hold_last_ms: snap.auth_hold_last_ms,
        auth_hold_p99_ms: snap.authoritative_hold_p99_ms(),
        auth_hold_max_ms: snap.auth_hold_max_ms,
        auth_last_bundle_bytes: snap.auth_bytes_last,
        auth_last_replay_writes: snap.auth_writes_last,
        auth_commits: [
            snap.auth_commits_finalized_total,
            snap.auth_commits_refused_total,
            snap.auth_commits_integrity_total,
            snap.auth_commits_quarantined_total,
        ],
        ipc_frames_shed: snap.ipc_frames_shed_total,
        ipc_write_timeouts: snap.ipc_write_timeouts_total,
        ipc_to_gw_overflow: snap.ipc_to_gw_overflow_total,
        ipc_hw_to_server: snap.ipc_queue_high_water_to_server,
        ipc_hw_to_gw: snap.ipc_queue_high_water_to_gw,
        ipc_links: snap.ipc_links,
        llm: llm_line,
        economy_journal: [
            snap.economy_journal_entries,
            snap.economy_journal_per_hour,
            snap.economy_replay_us_per_entry,
            snap.economy_replay_ms,
            snap.economy_fetch_ms,
        ],
    }
}

/// Render the wizard `@stats` report: uptime, object counts by kind,
/// connected count (with SSH/web split), loop lag (last/max/p99), store health
/// (read-only mode + last-write age + write-queue depth), NPC counts, today's
/// LLM spend, and db size. Pure over its inputs — unit-testable.
#[must_use]
pub fn render_stats(
    engine: &Engine,
    health: &StoreHealth,
    boot_unix: i64,
    now_unix: i64,
    extra: &StatsExtra,
) -> String {
    let w = &engine.world;
    let counts = object_counts(w);
    let total = counts.total();
    let connected = w.connected().count();
    let (lag_last, lag_max) = engine.loop_lag_ms();
    let uptime = (now_unix - boot_unix).max(0);
    let store = if health.mode.store_read_only() {
        "READ-ONLY (writes failing; mutations paused)".to_string()
    } else {
        "healthy".to_string()
    };
    let write_age = match health.last_write_unix {
        Some(at) => format!("{}s ago", (now_unix - at).max(0)),
        None => "no writes yet this boot".to_string(),
    };
    let mut text = String::from("Server statistics:");
    text.push_str(&format!(
        "\nUptime: {} (since unix {boot_unix})",
        human_duration(uptime)
    ));
    text.push_str(&format!(
        "\nObjects: {total} ({} rooms, {} exits, {} things, {} players, {} NPCs)",
        counts.rooms, counts.exits, counts.things, counts.players, counts.npcs
    ));
    if counts.going > 0 {
        text.push_str(&format!("\nPending destruction: {}", counts.going));
    }
    text.push_str(&format!(
        "\nConnected: {connected} (ssh {}, web {})",
        extra.ssh_conns, extra.web_conns
    ));
    text.push_str(&format!(
        "\nLoop lag: last {lag_last}ms, max {lag_max}ms, p99 {}ms",
        extra.lag_p99
    ));
    text.push_str(&format!(
        "\nStore: {store}; last write {write_age}; write-queue {}",
        extra.write_queue_depth
    ));
    // H3: the authoritative lane — the one pending-intent slot every
    // authoritative commit serializes on. Aggregates only.
    text.push_str(&format!(
        "\nAuthoritative lane: slot {}; hold last {}ms, p99 {}ms, max {}ms; last bundle {} bytes / {} writes; commits {} finalized, {} refused, {} integrity, {} quarantined",
        if extra.auth_slot_held { "HELD" } else { "free" },
        extra.auth_hold_last_ms,
        extra.auth_hold_p99_ms,
        extra.auth_hold_max_ms,
        extra.auth_last_bundle_bytes,
        extra.auth_last_replay_writes,
        extra.auth_commits[0],
        extra.auth_commits[1],
        extra.auth_commits[2],
        extra.auth_commits[3],
    ));
    // J1 (doc 12 D-12-1): the economy journal, what its replay cost, and the
    // projection the checkpoint decision keys on — the entry line derived from
    // the measured per-entry cost, and how far off it is at the current rate.
    let [entries, per_hour, us_per_entry, replay_ms, fetch_ms] = extra.economy_journal;
    let line_secs = i64::try_from(mush_economy::CHECKPOINT_REVIEW_SECS).unwrap_or(i64::MAX);
    let line_entries = if us_per_entry > 0 {
        line_secs.saturating_mul(1_000_000) / us_per_entry
    } else {
        0
    };
    let projection = if us_per_entry == 0 {
        "no replay measured".to_owned()
    } else if line_entries <= entries {
        "past the line".to_owned()
    } else if per_hour > 0 {
        format!(
            "~{} days away at this rate",
            (line_entries - entries) / per_hour.saturating_mul(24)
        )
    } else {
        "no posts since boot".to_owned()
    };
    text.push_str(&format!(
        "\nEconomy journal: {entries} entries, {per_hour}/h since boot; last replay {replay_ms}ms ({us_per_entry}us/entry; fetch {fetch_ms}ms); the {line_secs}s line is ~{line_entries} entries: {projection}"
    ));
    text.push_str(&format!(
        "\nNPC agents: {} active, {} thinking",
        extra.npc_active, extra.npc_thinking
    ));
    text.push_str(&format!(
        "\nNPC decisions: {} ready, {} leased, {} resolving",
        extra.npc_decisions[0], extra.npc_decisions[1], extra.npc_decisions[2]
    ));
    text.push_str(&format!(
        "\nIPC boundary: {} gateway link{}; {} frames shed, {} write timeouts, {} to_gw overflows; high-water to_server {}, to_gw {}",
        extra.ipc_links,
        if extra.ipc_links == 1 { "" } else { "s" },
        extra.ipc_frames_shed,
        extra.ipc_write_timeouts,
        extra.ipc_to_gw_overflow,
        extra.ipc_hw_to_server,
        extra.ipc_hw_to_gw
    ));
    if let Some(llm) = &extra.llm {
        text.push_str(&format!(
            "\nLLM today: {} in + {} out tokens, ${:.4} (kill switch: {})",
            llm.today_input,
            llm.today_output,
            llm.today_cost,
            if llm.globally_disabled {
                "ENGAGED"
            } else {
                "off"
            }
        ));
        for (feature, inp, out, cost) in &llm.features {
            text.push_str(&format!(
                "\n  {feature}: {inp} in + {out} out tokens, ${cost:.4}"
            ));
        }
    }
    if let (Some(definitions), Some(bridges), Some(migrations), Some(fingerprint)) = (
        engine.registry.definitions(),
        engine.registry.bridges(),
        engine.registry.definition_migrations(),
        engine.registry.mechanics_fingerprint(),
    ) {
        let summary = definitions.summary();
        text.push_str(&format!(
            "\nMechanics: {} definitions from {} owners, {} bridges, {} migrations; fingerprint {}",
            summary.definition_count,
            summary.owner_count,
            bridges.iter().len(),
            migrations.iter().len(),
            fingerprint,
        ));
    }
    for report in engine.registry.recovery_reports(w) {
        text.push_str(&format!(
            "\nRecovery {}: {} active, {} frozen, {} migrating, {} quarantined; {} incidents, {} migration plans{}",
            report.owner,
            report.active,
            report.frozen,
            report.migrating,
            report.quarantined,
            report.incident_count,
            report.migration_plan_count,
            if report.details_truncated { " (detail truncated)" } else { "" },
        ));
    }
    text.push_str(&format!(
        "\nDb size: {total} objects (next id #{})",
        w.next_id()
    ));
    text
}

/// `3661` → `"1h 1m 1s"`.
pub(super) fn human_duration(mut s: i64) -> String {
    let h = s / 3600;
    s %= 3600;
    let m = s / 60;
    s %= 60;
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

/// Hand off the staff ops the step's command handlers queued. Ban ops
/// (set/clear/list) go to the **sequential** [`staff_ops_loop`] so two
/// staff commands issued in order can never race in the store; `@dump`
/// rides the store-writer queue (ordered behind pending batches);
/// `@shutdown` returns `true` — the caller (the world loop) then steps
/// `ControlMsg::Shutdown` and exits. Every op is audit-logged with the
/// requesting actor.
pub(super) async fn run_staff_ops(
    _deps: &ServerDeps,
    gateway: &Gateway,
    writer: &StoreWriter,
    staff_ops: &mpsc::Sender<StaffWork>,
    ops: Vec<StaffOp>,
) -> bool {
    let mut shutdown = false;
    for op in ops {
        let actor = op.actor;
        match op.kind {
            StaffOpKind::SetBan {
                ref ip,
                until_unix,
                ref reason,
            } => {
                tracing::info!(%actor, %ip, ?until_unix, reason, "@siteban requested");
                if staff_ops.send(StaffWork::Op(op)).await.is_err() {
                    send_sys(gateway, actor, "@siteban failed: staff executor gone.");
                }
            }
            StaffOpKind::ClearBan { ref ip } | StaffOpKind::LiftTempban { ref ip, .. } => {
                tracing::info!(%actor, %ip, operation = ?op.kind, "@siteban pardon requested");
                if staff_ops.send(StaffWork::Op(op)).await.is_err() {
                    send_sys(
                        gateway,
                        actor,
                        "@siteban pardon failed: staff executor gone.",
                    );
                }
            }
            StaffOpKind::ListBans => {
                tracing::info!(%actor, "@siteban list requested");
                if staff_ops.send(StaffWork::Op(op)).await.is_err() {
                    send_sys(gateway, actor, "@siteban failed: staff executor gone.");
                }
            }
            StaffOpKind::NewPassword { target, .. } => {
                // Audit: actor + target only — the op's password never
                // reaches a log (Secret redacts even derived Debug).
                tracing::info!(%actor, %target, "@newpassword requested");
                if staff_ops.send(StaffWork::Op(op)).await.is_err() {
                    send_sys(gateway, actor, "@newpassword failed: staff executor gone.");
                }
            }
            StaffOpKind::AddSshKey {
                ref fingerprint, ..
            } => {
                // Audit: actor + fingerprint (public data). The raw pasted
                // line never reached this op — the command handler validated
                // and normalized it, queuing only the public key (S7).
                tracing::info!(%actor, fingerprint, "@sshkey/add requested");
                if staff_ops.send(StaffWork::Op(op)).await.is_err() {
                    send_sys(gateway, actor, "@sshkey failed: staff executor gone.");
                }
            }
            StaffOpKind::ListSshKeys { .. } => {
                tracing::info!(%actor, "@sshkey/list requested");
                if staff_ops.send(StaffWork::Op(op)).await.is_err() {
                    send_sys(gateway, actor, "@sshkey failed: staff executor gone.");
                }
            }
            StaffOpKind::DelSshKey { ref which, .. } => {
                tracing::info!(%actor, which, "@sshkey/del requested");
                if staff_ops.send(StaffWork::Op(op)).await.is_err() {
                    send_sys(gateway, actor, "@sshkey failed: staff executor gone.");
                }
            }
            StaffOpKind::NewChar { ref name, .. } => {
                // Audit: actor + the requested name (both public). The account
                // is resolved store-side in the executor (invariant N1).
                tracing::info!(%actor, name, "@newchar requested");
                if staff_ops.send(StaffWork::Op(op)).await.is_err() {
                    send_sys(gateway, actor, "@newchar failed: staff executor gone.");
                }
            }
            // The Discord identity family (D1): the operational trace carries
            // the actor and the target character only — never a code, never
            // an identity (the privileges audit is the record of the act).
            StaffOpKind::DiscordStatus { .. }
            | StaffOpKind::DiscordLinkMint { .. }
            | StaffOpKind::DiscordUnlinkSelf { .. }
            | StaffOpKind::DiscordSet { .. } => {
                tracing::info!(%actor, kind = ?op.kind, "@discord self-service op requested");
                if staff_ops.send(StaffWork::Op(op)).await.is_err() {
                    send_sys(gateway, actor, "@discord failed: staff executor gone.");
                }
            }
            // D3: the follow verbs are the gateway bridge's to answer; the
            // control crosses the seam fire-and-forget and the bridge tells the
            // character what happened on their own connections.
            StaffOpKind::DiscordFollow { actor: subject, on } => {
                tracing::info!(%actor, on, "@discord/follow requested");
                gateway.discord_control(
                    u64::from(subject.0),
                    if on {
                        DiscordControlOp::Follow
                    } else {
                        DiscordControlOp::Unfollow
                    },
                );
                send_sys(
                    gateway,
                    actor,
                    if on {
                        "@discord/follow: asked the Discord bot to follow you here; it answers \
                         on this screen and in your DM."
                    } else {
                        "@discord/unfollow: asked the Discord bot to detach."
                    },
                );
            }
            StaffOpKind::DiscordShow { target, .. }
            | StaffOpKind::DiscordUnlink { target, .. }
            | StaffOpKind::DiscordBan { target, .. } => {
                tracing::info!(%actor, %target, kind = ?op.kind, "@discord staff op requested");
                if staff_ops.send(StaffWork::Op(op)).await.is_err() {
                    send_sys(gateway, actor, "@discord failed: staff executor gone.");
                }
            }
            StaffOpKind::DiscordUnban { .. } => {
                tracing::info!(%actor, "@discord/unban requested");
                if staff_ops.send(StaffWork::Op(op)).await.is_err() {
                    send_sys(gateway, actor, "@discord failed: staff executor gone.");
                }
            }
            StaffOpKind::Dump => {
                tracing::info!(%actor, "@dump requested");
                if staff_ops
                    .send(StaffWork::Dump {
                        actor,
                        writer: writer.clone(),
                    })
                    .await
                    .is_err()
                {
                    send_sys(gateway, actor, "@dump failed: staff executor gone.");
                }
            }
            StaffOpKind::Shutdown => {
                tracing::info!(%actor, "@shutdown requested");
                shutdown = true;
            }
            // Local ops are partitioned out before this call.
            StaffOpKind::Boot { .. } | StaffOpKind::Ps | StaffOpKind::Stats => {
                debug_assert!(false, "local staff op reached the store executor path");
            }
        }
    }
    shutdown
}

/// Execute one `@newpassword`: argon2-hash on the blocking pool, find the
/// owning account by scanning the account docs (characters are few; the
/// store has no reverse index yet), update the stored hash, then revoke
/// every web token of that account (logout-everywhere) **and boot the
/// account's live web connections** — web transports are token-
/// authenticated, so they die with the tokens (SSH sessions are session-
/// authenticated and stay; boot them with `@boot` if needed). Returns the
/// confirmation text for the requesting wizard; delivers the target's
/// notification itself.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_newpassword(
    store: &dyn ObjectStore,
    gateway: &Gateway,
    world: &mpsc::WeakSender<WorldMsg>,
    target: ObjId,
    name: &str,
    by: &str,
    password: &str,
) -> String {
    let hash = {
        let password = password.to_string();
        match tokio::task::spawn_blocking(move || mush_config::accounts::hash_password(&password))
            .await
        {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => return format!("@newpassword failed: {e}."),
            Err(e) => return format!("@newpassword failed: hashing task failed: {e}."),
        }
    };
    let account = match store.list_accounts().await {
        Ok(accounts) => accounts
            .into_iter()
            .find(|a| a.characters.iter().any(|c| c.id == target)),
        Err(e) => return format!("@newpassword failed: {e}"),
    };
    let Some(account) = account else {
        return format!("@newpassword: {name} is not linked to any account.");
    };
    match store.set_account_password(&account.name, &hash).await {
        Ok(true) => {}
        Ok(false) => {
            return format!(
                "@newpassword failed: account {} vanished mid-update.",
                account.name
            );
        }
        Err(e) => return format!("@newpassword failed: {e}"),
    }
    // Logout-everywhere: every web session token of the account dies with
    // the old password. (Live SSH connections are session-authenticated
    // and stay; boot them separately with @boot if needed.)
    let revoked = match store.tokens_del_account(&account.name).await {
        Ok(n) => n,
        Err(e) => {
            // The password DID change; report the partial failure honestly.
            return format!(
                "@newpassword: password for {name} changed, but revoking web \
                 sessions failed: {e}"
            );
        }
    };
    tracing::info!(%target, account = %account.name, revoked, "@newpassword applied");
    // Notify BEFORE booting, so a web-connected target actually sees why
    // their session is about to close.
    send_sys(
        gateway,
        target,
        &format!("GAME: Your password has been changed by {by}."),
    );
    // The tokens are dead; so are the live connections they opened. Every
    // structured (web) connection on any of the account's characters is booted.
    let notice = OutputEvent::Disconnect {
        reason: RichText::tag(
            "system",
            format!("Your password has been changed by {by}; please log in again."),
        ),
    };
    // Split mode (M16b): the forked server holds no `SessionManager`, so it
    // cannot walk the account's connections. Emit ONE account/char-addressed
    // fire-and-forget `ToGateway::CloseWebSessions` — the gateway resolves each
    // character to its live structured connections on its own manager, delivers
    // the notice, drops them, and reports each `ConnDown(Boot)` back as inbound,
    // which the world steps exactly as the in-process walk below does. No
    // reverse query, and this runs on the staff-op task — never the world loop
    // (D4).
    if gateway.is_socket() {
        let chars: Vec<u64> = account
            .characters
            .iter()
            .map(|c| u64::from(c.id.0))
            .collect();
        gateway.close_web_sessions(chars, Some(notice), mush_ipc::DetachCause::Booted);
        return format!(
            "@newpassword: Password for {name} (account {}) changed; \
             {revoked} web session token{} revoked; any live web connections \
             were logged out.",
            account.name,
            if revoked == 1 { "" } else { "s" },
        );
    }
    // All-in-one: walk the shared manager (through the in-process sink) directly
    // — announce + bookkeeping through the world when the connection is still
    // up, then close each structured (web) connection.
    let world = world.upgrade().map(|tx| WorldHandle {
        tx,
        shutdown: None,
        direct_actions: None,
    });
    let chars: Vec<u64> = account
        .characters
        .iter()
        .map(|c| u64::from(c.id.0))
        .collect();
    let mut booted = 0u32;
    for (conn, char) in gateway.structured_conns_of(&chars) {
        let actor = ObjId(u32::try_from(char).unwrap_or(u32::MAX));
        if let Some(world) = &world {
            world
                .inbound(Inbound::ConnDown {
                    conn,
                    actor,
                    reason: DisconnectKind::Boot,
                })
                .await;
            world.barrier().await;
        }
        gateway.deliver(SendTarget::Conn(conn), Arc::new(notice.clone()));
        gateway.detach_conn_local(conn);
        booted += 1;
    }
    format!(
        "@newpassword: Password for {name} (account {}) changed; \
         {revoked} web session token{} revoked, {booted} live web \
         connection{} booted.",
        account.name,
        if revoked == 1 { "" } else { "s" },
        if booted == 1 { "" } else { "s" },
    )
}

/// Per-account cap on registered SSH public keys.
pub(super) const SSH_KEYS_PER_ACCOUNT: usize = 10;

/// Resolve the account that lists `actor` among its characters — the same
/// scan `@newpassword` does, but keyed on the CALLER, so `@sshkey` can only
/// ever read/modify the caller's OWN account (invariant S5). `Ok(None)` means
/// the character is linked to no account (a built-in like `#1` in a bare
/// world); `Err` is a store failure.
pub(super) async fn account_of_actor(
    store: &dyn ObjectStore,
    actor: ObjId,
) -> Result<Option<AccountRec>, String> {
    match store.list_accounts().await {
        Ok(accounts) => Ok(accounts
            .into_iter()
            .find(|a| a.characters.iter().any(|c| c.id == actor))),
        Err(e) => Err(e.to_string()),
    }
}

/// **One account record's immutable id** (16b cut F.4): the stored one, else
/// minted from `(name, created)` — the same derivation
/// [`crate::accounts_identity::hydrate`] uses, so a character linked at run time
/// and the same character re-hydrated at the next boot resolve to one id.
///
/// `None` only when the mint itself fails, which leaves the character unmapped
/// and failing closed (and logged by the caller).
pub(super) fn account_id_of(rec: &AccountRec) -> Option<mush_core::AccountId> {
    rec.id
        .or_else(|| mush_core::AccountId::mint(&rec.name, rec.created).ok())
}

/// The OpenSSH comment (everything after `type base64`), or a placeholder.
pub(super) fn ssh_comment(openssh: &str) -> &str {
    openssh
        .splitn(3, char::is_whitespace)
        .nth(2)
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or("(no comment)")
}

/// Execute `@sshkey/add`: resolve the caller's account, dedupe by fingerprint,
/// enforce the per-account cap, append the (already validated + normalized)
/// public key, and write it through. The gateway reads the account fresh per
/// auth, so the key authenticates on the very next `ssh -i` (invariant S4).
pub(super) async fn run_add_sshkey(
    store: &dyn ObjectStore,
    actor: ObjId,
    openssh: &str,
    fingerprint: &str,
) -> String {
    let account = match account_of_actor(store, actor).await {
        Ok(Some(a)) => a,
        Ok(None) => return "@sshkey: your character is not linked to an account.".to_string(),
        Err(e) => return format!("@sshkey/add failed: {e}"),
    };
    let mut keys = account.ssh_keys.clone();
    if keys.iter().any(|k| k.fingerprint == fingerprint) {
        return format!("@sshkey: that key is already registered ({fingerprint}).");
    }
    if keys.len() >= SSH_KEYS_PER_ACCOUNT {
        return format!(
            "@sshkey: your account already has the maximum of {SSH_KEYS_PER_ACCOUNT} keys; \
             remove one with @sshkey/del first."
        );
    }
    keys.push(SshKeyRec {
        openssh: openssh.to_string(),
        fingerprint: fingerprint.to_string(),
        added: unix_now_s(),
    });
    match store.set_account_ssh_keys(&account.name, &keys).await {
        Ok(true) => {
            // Audit: account + fingerprint (public); never the raw input (S7).
            tracing::info!(account = %account.name, fingerprint, "@sshkey/add applied");
            format!(
                "@sshkey: key {fingerprint} added. You can now log in with \
                 `ssh -i <your key> {}@game`.",
                account.name
            )
        }
        Ok(false) => format!(
            "@sshkey/add failed: account {} vanished mid-update.",
            account.name
        ),
        Err(e) => format!("@sshkey/add failed: {e}"),
    }
}

/// Execute `@sshkey/list`: render the caller's account's keys as
/// `index) SHA256:fp  <comment>  (added <date>)`.
pub(super) async fn run_list_sshkeys(store: &dyn ObjectStore, actor: ObjId) -> String {
    let account = match account_of_actor(store, actor).await {
        Ok(Some(a)) => a,
        Ok(None) => return "@sshkey: your character is not linked to an account.".to_string(),
        Err(e) => return format!("@sshkey/list failed: {e}"),
    };
    if account.ssh_keys.is_empty() {
        return "@sshkey: you have no registered SSH keys. Add one with \
                @sshkey/add <public key line>."
            .to_string();
    }
    let mut text = format!("SSH keys on account {}:", account.name);
    for (i, k) in account.ssh_keys.iter().enumerate() {
        text.push_str(&format!(
            "\n  {}) {}  {}  (added {})",
            i + 1,
            k.fingerprint,
            ssh_comment(&k.openssh),
            mush_mail::format_datetime(k.added),
        ));
    }
    text
}

/// Execute `@sshkey/del <index-or-fingerprint>`: remove one key from the
/// caller's account by 1-based list index or by exact `SHA256:...`
/// fingerprint, then write the remaining set back.
pub(super) async fn run_del_sshkey(store: &dyn ObjectStore, actor: ObjId, which: &str) -> String {
    let account = match account_of_actor(store, actor).await {
        Ok(Some(a)) => a,
        Ok(None) => return "@sshkey: your character is not linked to an account.".to_string(),
        Err(e) => return format!("@sshkey/del failed: {e}"),
    };
    let mut keys = account.ssh_keys.clone();
    // Prefer a 1-based index; fall back to a fingerprint match.
    let idx = match which.parse::<usize>() {
        Ok(n) if n >= 1 && n <= keys.len() => Some(n - 1),
        Ok(_) => None,
        Err(_) => keys.iter().position(|k| k.fingerprint == which),
    };
    let Some(idx) = idx else {
        return format!("@sshkey: no key matches {which:?}. Use @sshkey/list to see them.");
    };
    let removed = keys.remove(idx);
    match store.set_account_ssh_keys(&account.name, &keys).await {
        Ok(true) => {
            tracing::info!(account = %account.name, fingerprint = %removed.fingerprint, "@sshkey/del applied");
            format!("@sshkey: removed key {}.", removed.fingerprint)
        }
        Ok(false) => format!(
            "@sshkey/del failed: account {} vanished mid-update.",
            account.name
        ),
        Err(e) => format!("@sshkey/del failed: {e}"),
    }
}

/// Execute `@newchar <Name>`: create ONE more character on the CALLER's own
/// account (self-service; M20). Runs entirely off the world loop:
///
/// 1. Resolve the caller's OWN account with the same `list_accounts` scan
///    `@newpassword`/`@sshkey` use, keyed on `actor` — so it can only ever
///    touch the caller's own account (invariant N1).
/// 2. Enforce the `[game].max_characters` cap and an advisory world-name
///    pre-check BEFORE anything is created (N2). The cap check shares the same
///    accepted TOCTOU as the gateway `create_character` path: two concurrent
///    `@newchar`s at cap-1 on the same account can both pass (the append is
///    unconditional), overshooting the cap by one — it takes the account
///    holder racing themselves and the only damage is one extra character.
/// 3. Create the Player through the established durable, ack-gated server
///    create path ([`WorldHandle::create_player`], the same `WorldMsg::
///    CreatePlayer` route account creation uses) — which emits `ObjCreated`
///    (comsys auto-join) and waits for the store to acknowledge the creation
///    batch — then link it into the account doc (N3).
///
/// **Create-then-link ordering / orphan window:** identical to the gateway
/// `create_character` flow (`mush-gateway`'s `accounts::create_and_link`). The
/// Player is created (and durably persisted) BEFORE the account link; if the
/// link write then fails, a created-but-unlinked Player is left behind (the
/// store has no account-delete yet). That is the SAME accepted, loudly-logged
/// orphan window as `create_character` — no worse — and the pre-checks make it
/// a genuine store failure, not a normal path. **16b cut F.4** adds the one
/// thing that window now needs: the create turn told the live account oracle
/// about the character, so a failed link also sends
/// [`WorldHandle::unlink_player`] and the orphan is an orphan on both sides.
/// The confirmation is returned only after the link write returns (N3,
/// ack-gated). The caller is NOT switched to the new character (N4): they keep
/// playing their current one.
pub(super) async fn run_newchar(
    store: &dyn ObjectStore,
    world: &mpsc::WeakSender<WorldMsg>,
    actor: ObjId,
    name: &str,
    max_characters: usize,
) -> String {
    // (1) Resolve the caller's OWN account (N1). `account_of_actor` is the
    // same scan @sshkey/@newpassword use, keyed on the caller.
    let account = match account_of_actor(store, actor).await {
        Ok(Some(a)) => a,
        Ok(None) => return "@newchar: your character is not linked to an account.".to_string(),
        Err(e) => return format!("@newchar failed: {e}"),
    };
    // (2) Cap check BEFORE any create (N2).
    if account.characters.len() >= max_characters {
        return format!("@newchar: you already have the maximum of {max_characters} characters.");
    }
    // The world handle for the durable, ack-gated create path. A dropped world
    // (teardown) means we cannot create; fail closed rather than orphan.
    let Some(world) = world.upgrade().map(|tx| WorldHandle {
        tx,
        shutdown: None,
        direct_actions: None,
    }) else {
        return "@newchar failed: the world is shutting down.".to_string();
    };
    // Advisory name pre-check BEFORE create (N2); the create re-checks atomically.
    match world.name_free(name).await {
        Ok(true) => {}
        Ok(false) => return format!("@newchar: the name {name:?} is already taken."),
        Err(WorldReadError::Busy) => {
            return "@newchar: the world is committing another action; please retry shortly."
                .to_string();
        }
        Err(WorldReadError::Unavailable) => {
            return "@newchar failed: the world is unavailable.".to_string();
        }
    }
    // (3) Create through the established server create path — durable + ack-gated,
    // and it emits ObjCreated (comsys auto-join) exactly like account creation.
    // **16b cut F.4**: the caller's own account id rides with it, so the alt is
    // mapped to the SAME account before this reply prints — which is what makes
    // "one slot per account" hold for an alt made mid-session, and what stops the
    // alt being told its account is not on record.
    let id = match world.create_player(name, account_id_of(&account)).await {
        Ok(id) => id,
        // The pre-check raced another create; no second Player is made (the world
        // create rejects the duplicate atomically) — so this never orphans.
        Err(CreatePlayerErr::NameTaken) => {
            return format!("@newchar: the name {name:?} is already taken.");
        }
        Err(e) => return format!("@newchar failed: {e}"),
    };
    // Link it into the account doc. On failure the Player exists but is
    // unlinked — the SAME accepted, logged orphan window as create_character.
    match store.link_character(&account.name, id, name).await {
        Ok(()) => {
            tracing::info!(account = %account.name, %id, name, "@newchar created + linked");
            format!(
                "@newchar: created {name} ({id}) on your account. To play it, reconnect and \
                 pick it at the character-select screen, or `ssh {}+{name}@<host>` to jump \
                 straight in. It starts unapproved; chargen runs the first time you play it.",
                account.name
            )
        }
        Err(e) => {
            tracing::error!(
                account = %account.name, %id, name, error = %e,
                "@newchar: Player created but linking to the account failed; Player left unlinked"
            );
            // **16b cut F.4**: and the live oracle gives the mapping back, for
            // the same reason `create_and_link` does — the document it was
            // inserted against does not exist, so the alt must fail closed on
            // every account policy the way an unlinked character always has.
            match world.unlink_player(id).await {
                Ok(_) => {}
                Err(error) => tracing::error!(
                    account = %account.name, %id, name, %error,
                    "@newchar: the account oracle did not give the unlinked character back; it \
                     stays mapped until the next boot re-derives the map"
                ),
            }
            format!(
                "@newchar: created the character, but linking it to your account failed: {e}. \
                 Please contact staff."
            )
        }
    }
}

/// The sentence appended to every acknowledged `@siteban` set/clear.
///
/// The confirmation is gated on the STORE acknowledgement, never on the
/// gateway: this half's own [`IpGuard`] is mirrored immediately, but in a
/// forked deployment the guard that admits connections lives in the gateway
/// process and re-seeds itself from the store on its own clock
/// ([`SITEBAN_REFRESH`]). Staff who are told "banned" and then watch the
/// address connect for another few seconds should know why before they file it
/// as a bug.
pub(super) fn siteban_latency_note() -> String {
    format!(
        "Connection gateways re-read the ban list every {}s, so it can take that long to bite.",
        SITEBAN_REFRESH.as_secs()
    )
}

/// The sequential staff-op executor: one task, one queue, ops applied to
/// the store (and mirrored into the [`IpGuard`]) strictly in the order
/// staff issued them — a `@siteban/clear` can never lose to the `@siteban`
/// it follows. Confirmations are delivered to the requesting actor after
/// the store acknowledged. Exits when the world loop drops its sender.
pub(super) enum StaffWork {
    Op(StaffOp),
    Dump { actor: ObjId, writer: StoreWriter },
    Barrier(oneshot::Sender<Result<(), WorldReadError>>),
}

pub(super) async fn staff_ops_loop(
    store: Arc<dyn ObjectStore>,
    guard: Arc<IpGuard>,
    world: mpsc::WeakSender<WorldMsg>,
    mut rx: mpsc::Receiver<StaffWork>,
    gateway: Gateway,
) {
    // The staff executor's server->gateway output (confirmations, the target's
    // @newpassword notice) crosses the seam through this `gateway` sink — the
    // shared `SessionManager` in the all-in-one binary, the boundary socket in
    // the forked server. The session *control* ops @newpassword performs
    // (`structured_conns_of` / `detach_conn_local`) go through the same sink; in
    // split mode they resolve on the gateway process (this sink emits a
    // fire-and-forget `CloseWebSessions` instead).
    while let Some(work) = rx.recv().await {
        let op = match work {
            StaffWork::Barrier(reply) => {
                let _ = reply.send(Ok(()));
                continue;
            }
            StaffWork::Dump { actor, writer } => {
                let (done, done_rx) = oneshot::channel();
                let text = if writer.send(StoreJob::Bgsave { done }).await.is_err() {
                    "@dump failed: store writer gone.".to_owned()
                } else {
                    match done_rx.await {
                        Ok(Ok(())) => "@dump: snapshot acknowledged by the store.".to_owned(),
                        Ok(Err(error)) => format!("@dump failed: {error}"),
                        Err(_) => "@dump failed: store writer gone.".to_owned(),
                    }
                };
                send_sys(&gateway, actor, &text);
                continue;
            }
            StaffWork::Op(op) => op,
        };
        let actor = op.actor;
        let text = match op.kind {
            StaffOpKind::SetBan {
                ip,
                until_unix,
                reason,
            } => match store.set_ban(&ip.to_string(), until_unix, &reason).await {
                Ok(()) => {
                    guard.set_siteban(ip, until_unix);
                    let head = match until_unix {
                        None => format!("@siteban: {ip} banned permanently."),
                        Some(until) => format!("@siteban: {ip} banned until unix {until}."),
                    };
                    format!("{head} {}", siteban_latency_note())
                }
                Err(e) => format!("@siteban failed: {e}"),
            },
            StaffOpKind::ClearBan { ip } => match store.clear_ban(&ip.to_string()).await {
                Ok(removed) => {
                    guard.clear_siteban(ip);
                    if removed {
                        format!("@siteban: ban on {ip} lifted. {}", siteban_latency_note())
                    } else {
                        format!("@siteban: no ban on {ip}.")
                    }
                }
                Err(e) => format!("@siteban/clear failed: {e}"),
            },
            StaffOpKind::NewPassword {
                target,
                name,
                by,
                password,
            } => run_newpassword(&*store, &gateway, &world, target, &name, &by, &password.0).await,
            StaffOpKind::LiftTempban { ip, issued_unix_ms } => {
                let lift = mush_store::TempbanLift::new(ip, issued_unix_ms);
                match store.put_tempban_lift(lift.clone()).await {
                    Ok(()) => {
                        guard.apply_tempban_lift(
                            ip,
                            &lift.id,
                            lift.issued_unix_ms,
                            lift.expires_unix,
                        );
                        format!(
                            "@siteban/lift: password-tempban pardon published for {ip}'s network. Sitebans remain in force. {}",
                            siteban_latency_note()
                        )
                    }
                    Err(e) => format!("@siteban/lift failed: {e}"),
                }
            }
            StaffOpKind::ListBans => match store.get_bans().await {
                Ok(bans) if bans.is_empty() => "No active sitebans.".to_string(),
                Ok(mut bans) => {
                    bans.sort_by(|a, b| a.ip.cmp(&b.ip));
                    let mut text = String::from("Active sitebans:");
                    for b in bans {
                        let until = b
                            .until_unix
                            .map_or_else(|| "permanent".to_string(), |u| format!("until unix {u}"));
                        text.push_str(&format!("\n  {}  {}  {}", b.ip, until, b.reason));
                    }
                    text
                }
                Err(e) => format!("@siteban list failed: {e}"),
            },
            StaffOpKind::AddSshKey {
                actor: subject,
                openssh,
                fingerprint,
            } => run_add_sshkey(&*store, subject, &openssh, &fingerprint).await,
            StaffOpKind::ListSshKeys { actor: subject } => run_list_sshkeys(&*store, subject).await,
            StaffOpKind::DelSshKey {
                actor: subject,
                which,
            } => run_del_sshkey(&*store, subject, &which).await,
            StaffOpKind::NewChar {
                actor: subject,
                name,
                max_characters,
            } => run_newchar(&*store, &world, subject, &name, max_characters).await,
            StaffOpKind::DiscordStatus { actor: subject } => {
                run_discord_status(&*store, subject).await
            }
            StaffOpKind::DiscordLinkMint { actor: subject } => {
                run_discord_link_mint(&*store, subject).await
            }
            StaffOpKind::DiscordUnlinkSelf { actor: subject } => {
                run_discord_unlink(&gateway, &*store, subject, false).await
            }
            // Answered on the world loop through the gateway control (D3); it is
            // never queued to this executor, so this arm only keeps the match
            // honest.
            StaffOpKind::DiscordFollow { on, .. } => format!(
                "@discord/{}: routed to the Discord bridge.",
                if on { "follow" } else { "unfollow" }
            ),
            StaffOpKind::DiscordSet {
                actor: subject,
                setting,
            } => {
                // `quiet` also applies live to a follow in progress (D3); the
                // store carries the preference for the next one.
                let live = match setting {
                    crate::staff::DiscordSetting::Quiet(on) => Some(on),
                    _ => None,
                };
                let text = run_discord_set(&*store, subject, setting).await;
                if let Some(on) = live
                    && text.starts_with("@discord/set:")
                {
                    gateway.discord_control(u64::from(subject.0), DiscordControlOp::Quiet(on));
                }
                text
            }
            StaffOpKind::DiscordShow { actor: by, target } => {
                match discord_show_recheck(&world, by, target).await {
                    Ok(()) => run_discord_show(&*store, target).await,
                    Err(text) => text,
                }
            }
            StaffOpKind::DiscordUnlink { target, .. } => {
                run_discord_unlink(&gateway, &*store, target, true).await
            }
            StaffOpKind::DiscordBan {
                actor: by,
                target,
                reason,
            } => run_discord_ban(&gateway, &*store, by, target, &reason).await,
            StaffOpKind::DiscordUnban { target, .. } => run_discord_unban(&*store, target).await,
            // Dump/Shutdown and the local ops never enter this queue (see
            // run_staff_ops / run_local_staff_ops).
            StaffOpKind::Dump
            | StaffOpKind::Shutdown
            | StaffOpKind::Boot { .. }
            | StaffOpKind::Ps
            | StaffOpKind::Stats => continue,
        };
        send_sys(&gateway, actor, &text);
    }
    tracing::debug!("staff op executor drained and exiting");
}

// ---------------------------------------------------------------------------
// The Discord identity family (D1): store-side executors. Every reply below
// goes to the requesting actor alone; the only reply that renders an identity
// is `run_discord_show`, whose caller was privilege-gated and audited.
// ---------------------------------------------------------------------------

/// The display name of a character on an account record, or its dbref.
pub(super) fn account_char_name(account: &AccountRec, id: ObjId) -> String {
    account
        .characters
        .iter()
        .find(|c| c.id == id)
        .map_or_else(|| id.to_string(), |c| c.name.clone())
}

/// `@discord` — the caller's own link status, identity-free.
pub(super) async fn run_discord_status(store: &dyn ObjectStore, subject: ObjId) -> String {
    let account = match account_of_actor(store, subject).await {
        Ok(Some(a)) => a,
        Ok(None) => return "@discord: your character is not linked to an account.".to_string(),
        Err(e) => return format!("@discord failed: {e}"),
    };
    match &account.discord {
        None => "@discord: your account has no Discord link. `@discord/link` mints a code to \
                 bind one (see `help discord`)."
            .to_string(),
        Some(link) => {
            let relay = link.relay_character.map_or_else(
                || "your most recently connected character".to_string(),
                |c| account_char_name(&account, c),
            );
            let banned = match store.discord_ban_get(link.user_id).await {
                Ok(Some(_)) => "  BANNED: your Discord user may not link, follow, or relay.",
                _ => "",
            };
            format!(
                "@discord: linked since unix {}; auto-follow {}; quiet {}; relay character: \
                 {relay}.{banned}",
                link.linked_at,
                if link.auto_follow { "on" } else { "off" },
                if link.quiet { "on" } else { "off" },
            )
        }
    }
}

/// `@discord/link` — mint a one-shot code, store its hash with a TTL, and hand
/// the code to the caller alone. The code never enters a log.
pub(super) async fn run_discord_link_mint(store: &dyn ObjectStore, subject: ObjId) -> String {
    use mush_config::discord::{
        LINK_CODE_TTL_S, generate_link_code, link_code_hash, normalize_link_code,
    };
    let account = match account_of_actor(store, subject).await {
        Ok(Some(a)) => a,
        Ok(None) => return "@discord: your character is not linked to an account.".to_string(),
        Err(e) => return format!("@discord/link failed: {e}"),
    };
    if account.discord.is_some() {
        return "@discord/link: your account is already linked to a Discord user; \
                `@discord/unlink` first."
            .to_string();
    }
    let code = generate_link_code();
    let normalized = normalize_link_code(&code).expect("a minted code is well-formed");
    let hash = link_code_hash(&normalized);
    let expires = unix_now_s().saturating_add(LINK_CODE_TTL_S);
    match store
        .discord_code_put(
            hash,
            DiscordCodeRecord {
                account: account.name.clone(),
                expires,
            },
        )
        .await
    {
        Ok(()) => {
            tracing::info!(account = %account.name, "@discord/link code minted");
            format!(
                "@discord/link: your code is {code}. In Discord, send the game's bot a direct \
                 message: /link {code} — within {} minutes. The code works once; never share it.",
                LINK_CODE_TTL_S / 60
            )
        }
        Err(e) => format!("@discord/link failed: {e}"),
    }
}

/// `@discord/unlink` (self) and `@discord/unlink <player>` (staff): clear the
/// link and release the reverse index.
pub(super) async fn run_discord_unlink(
    gateway: &Gateway,
    store: &dyn ObjectStore,
    target: ObjId,
    by_staff: bool,
) -> String {
    let whose = if by_staff {
        "that account"
    } else {
        "your account"
    };
    let account = match account_of_actor(store, target).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return format!("@discord/unlink: {target} is not linked to an account.");
        }
        Err(e) => return format!("@discord/unlink failed: {e}"),
    };
    let Some(link) = account.discord.as_ref() else {
        return format!("@discord/unlink: {whose} has no Discord link.");
    };
    match store.set_account_discord(&account.name, None).await {
        Ok(true) => {}
        Ok(false) => {
            return format!(
                "@discord/unlink failed: account {} vanished mid-update.",
                account.name
            );
        }
        Err(e) => return format!("@discord/unlink failed: {e}"),
    }
    for character in &account.characters {
        gateway.discord_control(
            u64::from(character.id.0),
            mush_ipc::DiscordControlOp::Revoke,
        );
    }
    if let Err(e) = store.discord_user_release(link.user_id).await {
        // The doc is unlinked; the index is what stops a re-link elsewhere.
        return format!(
            "@discord/unlink: {whose} is unlinked, but releasing the index failed: {e}"
        );
    }
    tracing::info!(account = %account.name, by_staff, "@discord/unlink applied");
    format!("@discord/unlink: {whose} is no longer linked to Discord.")
}

/// `@discord/set` — a self-service preference on the caller's own link.
pub(super) async fn run_discord_set(
    store: &dyn ObjectStore,
    subject: ObjId,
    setting: crate::staff::DiscordSetting,
) -> String {
    use crate::staff::DiscordSetting;
    let account = match account_of_actor(store, subject).await {
        Ok(Some(a)) => a,
        Ok(None) => return "@discord: your character is not linked to an account.".to_string(),
        Err(e) => return format!("@discord/set failed: {e}"),
    };
    let Some(mut link) = account.discord.clone() else {
        return "@discord/set: your account has no Discord link yet (`@discord/link`).".to_string();
    };
    let what = match setting {
        DiscordSetting::RelayCharacter(Some(c)) => {
            // Only the caller's own characters may speak for the caller.
            if !account.characters.iter().any(|ch| ch.id == c) {
                return "@discord/set: the relay character must be one of your own characters."
                    .to_string();
            }
            link.relay_character = Some(c);
            format!("relay character set to {}", account_char_name(&account, c))
        }
        DiscordSetting::RelayCharacter(None) => {
            link.relay_character = None;
            "relay character reset to your most recently connected character".to_string()
        }
        DiscordSetting::AutoFollow(on) => {
            link.auto_follow = on;
            format!("auto-follow {}", if on { "on" } else { "off" })
        }
        DiscordSetting::Quiet(on) => {
            link.quiet = on;
            format!(
                "quiet {} (applies from your next follow)",
                if on { "on" } else { "off" }
            )
        }
    };
    match store.set_account_discord(&account.name, Some(&link)).await {
        Ok(true) => format!("@discord/set: {what}."),
        Ok(false) => format!(
            "@discord/set failed: account {} vanished mid-update.",
            account.name
        ),
        Err(e) => format!("@discord/set failed: {e}"),
    }
}

/// The execution-time authorization for `@discord/show`: the privilege was
/// tested on the world loop and the `use` audited **before** the op was
/// queued, and this executor is a queue hop later with no world borrow of its
/// own — so `@privilege/revoke` can land in between and the disclosure would
/// still go out. Round-trip one `WorldMsg` (the `@newpassword` precedent) and
/// let the world loop answer.
///
/// Fail closed: a revocation, a busy world (a commit is pending) and a gone
/// world all refuse. The world loop writes the second `Denied` audit row on
/// the revocation; `Err` here is a refusal to disclose, never a panic — this
/// module denies `clippy::unreachable` and the loop must not be lost over a
/// staff op.
pub(super) async fn discord_show_recheck(
    world: &mpsc::WeakSender<WorldMsg>,
    actor: ObjId,
    target: ObjId,
) -> Result<(), String> {
    let Some(world) = world.upgrade().map(|tx| WorldHandle {
        tx,
        shutdown: None,
        direct_actions: None,
    }) else {
        return Err("@discord/show: the world is shutting down; nothing was shown.".to_string());
    };
    match world
        .recheck_privilege(
            actor,
            Privileges::DISCORD_IDENTITY,
            Some(target),
            "@discord/show",
        )
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => {
            tracing::warn!(
                %actor, %target,
                "@discord/show refused at execution: the privilege was revoked after the command"
            );
            Err("@discord/show: your DISCORD_IDENTITY privilege was revoked before the                  look could be answered. Permission denied."
                .to_string())
        }
        Err(e) => Err(format!(
            "@discord/show: the privilege could not be re-checked ({e}); nothing was shown."
        )),
    }
}

/// `@discord/show <player>` — the one surface that renders the identity. The
/// caller held `DISCORD_IDENTITY` and the `use` is already in the audit.
pub(super) async fn run_discord_show(store: &dyn ObjectStore, target: ObjId) -> String {
    let account = match account_of_actor(store, target).await {
        Ok(Some(a)) => a,
        Ok(None) => return format!("@discord/show: {target} is not linked to an account."),
        Err(e) => return format!("@discord/show failed: {e}"),
    };
    let who = account_char_name(&account, target);
    let Some(link) = account.discord.as_ref() else {
        return format!(
            "@discord/show: {who} (account {}) has no Discord link.",
            account.name
        );
    };
    let relay = link.relay_character.map_or_else(
        || "most recent".to_string(),
        |c| account_char_name(&account, c),
    );
    let ban = match store.discord_ban_get(link.user_id).await {
        Ok(Some(b)) => format!(
            "  BANNED by {} at unix {}: {}",
            b.by_account, b.at, b.reason
        ),
        Ok(None) => String::new(),
        Err(e) => format!("  (ban lookup failed: {e})"),
    };
    format!(
        "@discord/show: {who} (account {}): Discord user {}, linked unix {}, auto-follow {}, \
         relay character {relay}.{ban}",
        account.name,
        link.user_id,
        link.linked_at,
        if link.auto_follow { "on" } else { "off" },
    )
}

/// `@discord/ban <player>=<reason>` — ban the target's linked Discord user by
/// snowflake (the link stays so `@discord/unban <player>` can find it; the
/// bridge refuses link, follow and relay while the ban stands).
pub(super) async fn run_discord_ban(
    gateway: &Gateway,
    store: &dyn ObjectStore,
    by: ObjId,
    target: ObjId,
    reason: &str,
) -> String {
    let account = match account_of_actor(store, target).await {
        Ok(Some(a)) => a,
        Ok(None) => return format!("@discord/ban: {target} is not linked to an account."),
        Err(e) => return format!("@discord/ban failed: {e}"),
    };
    let who = account_char_name(&account, target);
    let Some(link) = account.discord.as_ref() else {
        return format!("@discord/ban: {who} has no linked Discord user to ban.");
    };
    let by_account = match account_of_actor(store, by).await {
        Ok(Some(a)) => a.name,
        _ => by.to_string(),
    };
    match store
        .discord_ban_put(DiscordBanRec {
            user_id: link.user_id,
            by_account,
            reason: reason.to_string(),
            at: unix_now_s(),
        })
        .await
    {
        Ok(()) => {
            for character in &account.characters {
                gateway.discord_control(
                    u64::from(character.id.0),
                    mush_ipc::DiscordControlOp::Revoke,
                );
            }
            tracing::info!(account = %account.name, "@discord/ban applied");
            format!(
                "@discord/ban: {who}'s Discord user is banned — they can no longer link, \
                 follow, or relay. `@discord/unban {who}` lifts it."
            )
        }
        Err(e) => format!("@discord/ban failed: {e}"),
    }
}

/// `@discord/unban <player>|<snowflake>`.
pub(super) async fn run_discord_unban(
    store: &dyn ObjectStore,
    target: crate::staff::DiscordBanTarget,
) -> String {
    use crate::staff::DiscordBanTarget;
    let (user_id, who) = match target {
        DiscordBanTarget::User(id) => (id, "that Discord user".to_string()),
        DiscordBanTarget::Player(t) => {
            let account = match account_of_actor(store, t).await {
                Ok(Some(a)) => a,
                Ok(None) => return format!("@discord/unban: {t} is not linked to an account."),
                Err(e) => return format!("@discord/unban failed: {e}"),
            };
            let who = account_char_name(&account, t);
            let Some(link) = account.discord.as_ref() else {
                return format!(
                    "@discord/unban: {who} has no linked Discord user; if they unlinked while \
                     banned, unban by Discord user id."
                );
            };
            (link.user_id, format!("{who}'s Discord user"))
        }
    };
    match store.discord_ban_del(user_id).await {
        Ok(true) => format!("@discord/unban: {who} is no longer banned."),
        Ok(false) => format!("@discord/unban: {who} was not banned."),
        Err(e) => format!("@discord/unban failed: {e}"),
    }
}

/// Send one system line to an actor as a [`ToGateway::Deliver`](mush_ipc::ToGateway)
/// through the seam (staff-op confirmations, store-failure warnings).
pub(super) fn send_sys(gateway: &Gateway, actor: ObjId, text: &str) {
    gateway.deliver(
        SendTarget::Char(u64::from(actor.0)),
        Arc::new(OutputEvent::Sys(RichText::tag("system", text))),
    );
}
