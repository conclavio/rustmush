//! **The arena seam's world half, through the real world task** (16b cut T.2,
//! T4; doc 16 §3.1 "The seam is seven messages", §4 "Ledger events", §5).
//!
//! Everything cut T.2 claims about the world side is a claim about the *loop*:
//! that the arena's egress is drained once per iteration without blocking, that
//! each message re-enters as a world command with a `Ctx`, that a frame leaves as
//! an `arena.cockpit` keyframe through the loop's ordinary output path addressed as
//! `SendTarget::Char`, and that a chunk lands in `log:match.<id>` under the
//! tacsim module's own namespace with its acknowledgement going back to the arena.
//! None of that can be shown by calling a function: it is shown by starting the
//! real loop from
//! [`spawn_persist_with_gateway`](mush_server::world_task::spawn_persist_with_gateway),
//! starting a match inside it, and reading what came out of the gateway and what
//! went into the store.
//!
//! The rig is the one `world_panic_firewall.rs` uses: a recording gateway, an
//! in-memory store, and one test module whose single verb does on the world task
//! exactly what cut T.3's `@arena/start` will do — it calls
//! [`tacsim::arena_bridge::open_match`]. The units are bare object ids: custody
//! (the `deployed` marker, the frozen placement, the check-in) is T.3's, and
//! nothing in T.2 reads a unit record.

use std::sync::Arc;
use std::time::Duration;

use mush_config::ipguard::IpGuard;
use mush_config::metrics::Metrics;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, DefinitionDigest, DefinitionId, DefinitionKey,
    DefinitionKind, Engine, GOD, GameModule, Inbound, ModError, ObjId, OwnerId, Registrar, World,
};
use mush_ipc::{DetachCause, SendTarget};
use mush_proto::OutputEvent;
use mush_server::gateway_sink::GatewaySink;
use mush_server::quarantine::QuarantineSpool;
use mush_server::staff::StaffQueue;
use mush_server::world_task::{
    self, Gateway, PersistHandles, ServerDeps, spawn_persist_with_gateway,
};
use mush_sim::data::Pose;
use mush_store::{InMemoryStore, ObjectStore};
use mush_tactics::{
    CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec, TacticalUnit,
    UnitSpec,
};
use smol_str::SmolStr;
use tacsim::arena::state::{
    ArenaDefinition, ArenaTerrain, CustodyOut, CustodyUnit, EntryZone, Pilot, Team, TerrainCell,
    TrackedRules,
};
use tacsim::arena_task::MatchId;
use tacsim::config::{ArenaConfig, MatchConfig};

/// The match this test runs.
const MATCH: MatchId = MatchId(4_242);
/// The one unit. Not a world object: T.2 reads no unit record.
const UNIT: ObjId = ObjId(9_001);
/// The connection God is attached on.
const CONN: u64 = 0x4242;

// ---------------------------------------------------------------------------
// Rig
// ---------------------------------------------------------------------------

/// A sink that keeps every delivered `(target, event)` so a proof can ask what a
/// connected character was actually sent.
#[derive(Default)]
struct RecordingGateway {
    sent: std::sync::Mutex<Vec<(SendTarget, Arc<OutputEvent>)>>,
}

impl RecordingGateway {
    /// Every `arena.cockpit` keyframe, with the target it went to.
    fn frames(&self) -> Vec<(SendTarget, serde_json::Value)> {
        self.sent
            .lock()
            .expect("recording gateway mutex")
            .iter()
            .filter_map(|(target, event)| match event.as_ref() {
                OutputEvent::Panel { feed, kind, data }
                    if feed.as_str() == tacsim::arena_bridge::FRAME_FEED
                        && *kind == mush_proto::PanelFrameKind::Keyframe =>
                {
                    Some((target.clone(), data.clone()))
                }
                _ => None,
            })
            .collect()
    }
}

impl GatewaySink for RecordingGateway {
    fn transmit(&self, sends: Vec<(SendTarget, Arc<OutputEvent>)>) {
        let mut log = self.sent.lock().expect("recording gateway mutex");
        for send in sends {
            log.push(send);
        }
    }

    fn set_flood_exempt(&self, _char: u64, _exempt: bool) {}

    fn is_socket(&self) -> bool {
        false
    }

    fn close_session(&self, _char: u64, _cause: DetachCause) {}

    fn close_web_sessions(
        &self,
        _chars: Vec<u64>,
        _notice: Option<OutputEvent>,
        _cause: DetachCause,
    ) {
    }

    fn close_char_connections(&self, _char: u64) -> Vec<u64> {
        Vec::new()
    }

    fn structured_conns_of(&self, _chars: &[u64]) -> Vec<(u64, u64)> {
        Vec::new()
    }

    fn detach_conn_local(&self, _conn: u64) {}

    fn session_counts(&self) -> (i64, i64) {
        (0, 0)
    }
}

// ---------------------------------------------------------------------------
// The custody fixture
// ---------------------------------------------------------------------------

fn dref(kind: &str, id: &str) -> DefinitionRef {
    DefinitionRef::new(
        DefinitionKey::new(
            OwnerId::new("tacsim").expect("a canonical owner"),
            DefinitionKind::new(kind).expect("a canonical kind"),
            DefinitionId::new(id).expect("a canonical id"),
        ),
        1,
        DefinitionDigest::from_bytes([0_u8; 32]),
    )
    .expect("version 1 is not zero")
}

fn rules() -> RulesValue {
    RulesValue::new(dref("schema", "tacsim.arena.bridge"), serde_json::json!({}))
        .expect("an empty rules blob")
}

fn tactical_unit(id: ObjId) -> TacticalUnit {
    let spec = UnitSpec {
        definition: dref("unit", "tacsim.unit.line_tank"),
        sections: vec![SectionSpec {
            id: SectionId::new("hull").expect("a canonical section"),
            condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
            facets: vec![FacetSpec {
                id: FacetId::new("front").expect("a canonical facet"),
                cells: vec![CellSpec {
                    id: mush_tactics::CellId::new("armor").expect("a cell"),
                    min: 0,
                    max: 10,
                    init: 10,
                }],
            }],
            slots: Vec::new(),
            cells: vec![CellSpec {
                id: mush_tactics::CellId::new("internals").expect("a cell"),
                min: 0,
                max: 8,
                init: 8,
            }],
        }],
        unit_cells: Vec::new(),
        components: Vec::new(),
        capabilities: Vec::new(),
        mobility: None,
        rules: rules(),
    };
    TacticalUnit::from_spec(&spec, id).expect("a one-section unit")
}

fn ground() -> TerrainCell {
    TerrainCell {
        class: mush_atlas::TerrainClass::Ground,
        elev_cm: 0,
        move_cost: 100,
    }
}

/// One tank, God in its cockpit, on an 8 by 8 window.
fn custody() -> CustodyOut {
    CustodyOut {
        definition: ArenaDefinition {
            id: SmolStr::new("tacsim.arena.bridge"),
            terrain: ArenaTerrain::uniform(8, 8, 1_000, ground()).expect("the test window"),
            entry_zones: vec![EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: 7,
            }],
            // 16b cut N.1: no authored furniture; this fixture's unit is a pilot's.
            key_spots: Vec::new(),
            waypoints: tacsim::arena::state::Waypoints::default(),
        },
        units: vec![CustodyUnit {
            id: UNIT,
            team: Team::Red,
            pose: Pose::at(1_500, 1_500, 0, rules()),
            mobility: TrackedRules {
                max_speed_cms: 1_000,
                accel_cms2: 200,
                brake_cms2: 400,
                turn_rate_mdeg_s: 30_000,
                reverse_factor_pct: 40,
            },
            sensor_height_cm: 150,
            sensor: None,
            weapons: Vec::new(),
            profile: tacsim::arena::resolution::UnitProfile::default(),
            gunnery: 0,
            controller: tacsim::arena::state::Controller::Pilot,
            // 16b cut Q.1: no pod behind it.
            slot: 0,
            unit: tactical_unit(UNIT),
        }],
        pilots: vec![Pilot {
            unit: UNIT,
            pilot: GOD,
            station: SmolStr::new("cockpit"),
            epoch: 1,
        }],
        resolution: tacsim::match_::stock_resolution(),
        seed: 7_777,
    }
}

/// How many unacknowledged chunks the arena tolerates before it voids the match
/// (doc 16 §3.1's bounded egress). Deliberately tiny: it is what makes the
/// acknowledgement round trip *observable* rather than merely plausible — past
/// this many unacked chunks the match is `Void`, so a match still running after
/// [`CHUNKS_PAST_THE_BOUND`] chunks have landed can only be a match whose
/// acknowledgements are reaching it.
const MAX_UNFLUSHED: u32 = 4;

/// The chunk index the proof waits for: one past `MAX_UNFLUSHED + 1` chunks, so
/// the arena would certainly have voided by now if nothing were being
/// acknowledged. `mush_core::Ledger` chunks are 0-based and one arena chunk fills
/// one of them, so index 5 is the sixth.
const CHUNKS_PAST_THE_BOUND: u64 = 5;

/// Fast and chatty, so the proof finishes inside a couple of seconds: a 50 ms
/// pulse, a 1 ms `position_summary` cadence (which fills 16 KiB chunks quickly)
/// and the tiny egress bound above. Nothing else is tuned.
fn arena_cfg() -> ArenaConfig {
    let mut cfg = ArenaConfig::default();
    cfg.tempo.quiet_ms = 50;
    cfg.tempo.alert_ms = 50;
    cfg.tempo.engaged_ms = 50;
    cfg.ledger.summary_ms = 1;
    cfg.ledger.max_unflushed_chunks = MAX_UNFLUSHED;
    cfg
}

// ---------------------------------------------------------------------------
// The one test module: `@arenastart`, which is what T.3's `@arena/start` will do
// ---------------------------------------------------------------------------

struct ArenaStarter;

impl GameModule for ArenaStarter {
    fn name(&self) -> &'static str {
        "arenabridgetest"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
        reg.command(
            CommandSpec {
                name: "@arenastart".into(),
                shape: ArgShape::None,
                help: "test: check one unit out into a match".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                tacsim::arena_bridge::open_match(
                    ctx,
                    MATCH,
                    custody(),
                    &arena_cfg(),
                    &MatchConfig::default(),
                    &tacsim::config::SensorsConfig::default(),
                )
                // 16b cut V.2: the answer is the callsigns, which this rig
                // does not read.
                .map(|_callsigns| ())
                .map_err(|error| CmdError::msg(error.to_string()))
            }),
        )?;
        Ok(())
    }
}

fn boot(store: Arc<dyn ObjectStore>, gateway: Arc<RecordingGateway>) -> PersistHandles {
    let engine = Engine::with_modules(
        World::new_minimal(),
        world_task::base_registry(),
        vec![
            Box::new(tacsim::TacsimRuleset::default()),
            Box::new(ArenaStarter),
        ],
    )
    .expect("the arena-bridge test composition");
    spawn_persist_with_gateway(
        engine,
        ServerDeps {
            store,
            guard: Arc::new(IpGuard::new()),
            staff: Arc::new(StaffQueue::new()),
            llm: None,
            metrics: Arc::new(Metrics::new()),
            quarantine_spool: QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
        move |_| -> Gateway { gateway },
    )
}

/// Poll `check` until it answers, or fail after `bound`.
async fn until(bound: Duration, label: &str, mut check: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + bound;
    while tokio::time::Instant::now() < deadline {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{label} did not happen within {bound:?}");
}

// ---------------------------------------------------------------------------
// The proof
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_running_arenas_frames_reach_its_pilot_and_its_chunks_land_in_the_match_log() {
    let store = Arc::new(InMemoryStore::new());
    let gateway = Arc::new(RecordingGateway::default());
    let handles = boot(
        Arc::clone(&store) as Arc<dyn ObjectStore>,
        Arc::clone(&gateway),
    );

    // The shared table the host read out of the game's own state slot. Its
    // presence is the first half of the wiring: a composition with a game layer
    // gets one, and the loop's egress arm is armed from it.
    let arenas = handles
        .arenas
        .clone()
        .expect("the tacsim layer composed, so the host found its arena table");
    assert!(
        arenas.live_ids().is_empty(),
        "no match is running before one is started"
    );

    // God connects, so there is a pilot the loop can deliver a frame to.
    assert!(
        handles
            .handle
            .inbound(Inbound::ConnUp {
                conn: CONN,
                actor: GOD,
                caps_structured: true,
                from_linkdead: false,
            })
            .await
    );
    assert!(handles.handle.barrier().await, "the connect settles");

    // Start the match, on the world task, the way cut T.3's verb will.
    assert!(
        handles
            .handle
            .inbound(Inbound::Cmd {
                conn: CONN,
                actor: GOD,
                line: "@arenastart".to_string(),
            })
            .await
    );
    assert!(handles.handle.barrier().await, "the start settles");
    assert_eq!(
        arenas.live_ids(),
        vec![MATCH],
        "the match is in the shared table"
    );

    // ---- (a) the frames reach the pilot as `arena.cockpit` keyframes ----
    until(Duration::from_secs(5), "a cockpit frame", || {
        !gateway.frames().is_empty()
    })
    .await;
    let frames = gateway.frames();
    let (target, data) = frames.first().cloned().expect("at least one frame");
    assert_eq!(
        target,
        SendTarget::Char(u64::from(GOD.0)),
        "a frame is addressed to the pilot's character through the ordinary output path"
    );
    assert_eq!(
        data.get("unit").and_then(serde_json::Value::as_u64),
        Some(u64::from(UNIT.0)),
        "and it is that pilot's own unit: {data}"
    );
    assert_eq!(
        data.get("match").and_then(serde_json::Value::as_u64),
        Some(MATCH.0),
        "stamped with the match: {data}"
    );
    assert!(
        data.get("tempo")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "carrying the tempo the arena is running at: {data}"
    );

    // ---- (b) the chunks land in `log:match.<id>` under tacsim's namespace ----
    let key = format!("log:{}:0", tacsim::arena_bridge::match_log_id(MATCH));
    let chunk = {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(value) = store
                .module_json_get("tacsim", &key)
                .await
                .expect("the store read")
            {
                break value;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "no match ledger chunk reached the store at mod:tacsim:{key}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    let entries = chunk
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        !entries.is_empty(),
        "the chunk document holds the ledger entries: {chunk}"
    );
    let lines = entries
        .first()
        .and_then(|entry| entry.get("value"))
        .and_then(|value| value.get("lines"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    assert!(
        lines.contains("match_started"),
        "and the entry carries the arena's own bytes, exactly: {lines:?}"
    );

    // ---- (c) the acknowledgement round-trips ----
    //
    // The handler queued an ack for every chunk it appended; the host stamped it
    // with the batch's serial; the writer's durable-commit high-water reached it,
    // and the loop offered it to the arena. The proof is the **arena's own**
    // behaviour: its egress tolerates `MAX_UNFLUSHED` unacknowledged chunks and
    // voids the match past that (doc 16 §3.1), so a match that has streamed
    // `CHUNKS_PAST_THE_BOUND + 1` chunks and is *still running* is a match whose
    // acknowledgements came back.
    let past_the_bound = format!(
        "log:{}:{CHUNKS_PAST_THE_BOUND}",
        tacsim::arena_bridge::match_log_id(MATCH)
    );
    {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while store
            .module_json_get("tacsim", &past_the_bound)
            .await
            .expect("the store read")
            .is_none()
        {
            assert!(
                arenas.is_live(MATCH),
                "the match voided before it got past {MAX_UNFLUSHED} unacknowledged chunks, so \
                 nothing was being acknowledged"
            );
            assert!(
                tokio::time::Instant::now() < deadline,
                "the match log never reached chunk {CHUNKS_PAST_THE_BOUND}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    assert!(
        arenas.is_live(MATCH),
        "and it is still running after {} chunks, which is only possible with the \
         acknowledgement round trip",
        CHUNKS_PAST_THE_BOUND + 1
    );
    // Nothing is left waiting once the writer has caught up.
    until(Duration::from_secs(10), "every chunk acknowledged", || {
        arenas.pending_ledger_acks() == 0
    })
    .await;

    // ---- (d) `@arena/status` reads the mirror the egress built ----
    assert!(
        handles
            .handle
            .inbound(Inbound::Cmd {
                conn: CONN,
                actor: GOD,
                line: "@arena/status".to_string(),
            })
            .await
    );
    assert!(handles.handle.barrier().await, "the status settles");
    let said: Vec<String> = gateway
        .sent
        .lock()
        .expect("recording gateway mutex")
        .iter()
        .map(|(_, event)| format!("{event:?}"))
        .collect();
    assert!(
        said.iter()
            .any(|line| line.contains(&format!("match {MATCH}")) && line.contains("running")),
        "the staff read names the running match: {said:?}"
    );
    assert!(
        said.iter()
            .any(|line| line.contains(" appended") && !line.contains("chunks 0 appended")),
        "and a non-zero chunk count, built by the egress handler as the chunks arrived: {said:?}"
    );

    // ---- shutdown: the world first, then the arena tasks, then the writer ----
    assert!(
        handles
            .handle
            .inbound(Inbound::Control(mush_core::ControlMsg::Shutdown))
            .await
    );
    let _ = handles.world_join.await.expect("the world task joins");
    world_task::arena_shutdown(handles.arenas).await;
    assert!(
        arenas.live_ids().is_empty(),
        "the shutdown tail stopped and joined every arena task"
    );
    let _ = handles.writer_join.await;
    let _ = handles.outbox_join.await;
}
