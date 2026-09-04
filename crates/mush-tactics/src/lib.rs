//! # mush-tactics — the standing seam test (design doc 04 §6.3)
//!
//! A **toy** tactical module whose only job is to exercise every core
//! extension point the real engine (M10+) will need, so those seams can
//! never silently rot (PLAN: "tactics stub crate green in CI — stays green
//! forever after"; treaty ruling 4's standing acceptance test). It is not
//! wired into a production `mushd`; its integration tests construct it via
//! [`Engine::with_modules`](mush_core::engine::Engine::with_modules) —
//! adding it touches config and `Cargo.toml`, zero core code (checklist
//! item 6).
//!
//! Seams exercised, one per checklist item:
//!
//! 1. **Sub-second periodic tick** — the `pulse` tick at
//!    [`TickCadence::Fast`]`(250ms)`, re-armed by the scheduler each fire.
//! 2. **Scene lifecycle + dynamic per-scene ticker** — `+btest/start`
//!    opens a `tactics.battle` [`Scene`](mush_core::Scene) and starts the
//!    `battle` [`TickCadence::Scene`] tick for it; `+btest/stop` ends the
//!    scene (the ticker dies with it); a `SceneEnded` hook cleans up.
//! 3. **`RulesetStateStore` + `MapBlob`** — every battle tick bumps each
//!    member's `heat` in the per-`(scene, obj)` state store (each change
//!    persists a whole-scene snapshot [`ModuleOp`](mush_core::ModuleOp)),
//!    and `+btest/start` stores a small hex map blob core never interprets.
//! 4. **Panel keyframe/delta frames** — the `tactics.map` feed emits one
//!    keyframe at battle start and a delta per tick.
//! 5. **`MoveAttempt` veto** — rooms flagged with the registered
//!    [`HEXLOCK_MARKER`] marker refuse entry (exit traversal *and*
//!    `@teleport`) with a themed message.
//!
//! Real hex math (`hexx`), heat curves, and hit tables stay entirely out of
//! scope until the real module lands.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use mush_core::module::{
    EventHook, GameModule, HookPoint, ModError, ModuleBootData, ModuleCtx, Registrar, TickCadence,
    WeatherQuery, WorldEvent, WorldEventKind, WxSnapshot,
};
use mush_core::{
    ArgShape, Audience, CmdError, CommandSpec, Ctx, ModuleOp, ModuleWrite, ObjId, PanelFeedHandle,
    SceneId, World,
};
use mush_proto::RichText;
use smol_str::SmolStr;

pub mod mobility;
pub mod module;
pub mod unit;

pub use mobility::{
    LocomotionId, MediumId, MobilityError, MobilityMode, MobilityModeId, MobilityProfile,
    MobilityRegistry, MobilityTag, MobilityVocabulary, ModeRules, ModeState,
};
pub use module::{
    BRACE_ACTION_ID, BRACED_OUTCOME_ID, MAX_UNIT_BINDING_REPAIRS, MAX_UNIT_RECORD_BYTES,
    STANCE_STATE_FAMILY, TacticalStance, TacticsModule, UNIT_BINDING_VERSION, UNIT_RECORD_VERSION,
    UnitBindingRepair, UnitRecordStatus, UnitStore,
};
pub use unit::{
    CapabilityId, CapabilityInit, CapabilityKey, CellAddr, CellId, CellSpec, ComponentId,
    ComponentSpec, ConditionId, DefinitionRef, FacetId, FacetSpec, InstallPoint, InstallSlotId,
    InstalledComponent, Journal, JournalEntry, LegacyUnitV1Decoded, LegacyUnitV1Error,
    MAX_UNIT_CAPABILITIES, MAX_UNIT_CATALOG_BYTES, MAX_UNIT_CATALOG_DEFINITIONS, MAX_UNIT_CELLS,
    MAX_UNIT_COMPONENTS, MAX_UNIT_FACETS, MAX_UNIT_MOBILITY_MODES, MAX_UNIT_MUTATION_OPERATIONS,
    MAX_UNIT_SECTIONS, MAX_UNIT_SLOTS, MAX_UNIT_SPEC_BYTES, RulesValue, ScalarCell, SectionId,
    SectionSpec, SectionState, TacticalUnit, UnitCatalog, UnitCatalogBuilder, UnitError,
    UnitMutationOp, UnitMutationPlan, UnitSpec, UnitSpecBody, apply, decode_legacy_unit_v1,
};

/// The module id (also its persistence namespace).
// NOTE (M33): the REAL `TacticsModule` (module.rs) owns NS "tactics"; this
// standing seam-test stub moved to "tacseam" so both can load in one Engine.
pub const NS: &str = "tacseam";

/// The registered marker flag: a room carrying it refuses all movement
/// into it (`@set here=tacseam:hexlock`).
pub const HEXLOCK_MARKER: &str = "tacseam:hexlock";

/// The scene kind `+btest/start` opens.
pub const SCENE_KIND: &str = "tacseam.battle";

/// The panel feed id (design §6.2's reserved `tactics.map`).
pub const MAP_FEED: &str = "tacseam.map";

/// The sub-second periodic tick's tag and period.
pub const PULSE_TICK: &str = "pulse";
/// See [`PULSE_TICK`].
pub const PULSE_PERIOD: Duration = Duration::from_millis(250);

/// The per-scene battle tick's tag and period.
pub const BATTLE_TICK: &str = "battle";
/// See [`BATTLE_TICK`].
pub const BATTLE_PERIOD: Duration = Duration::from_millis(250);

/// The veto message hexlocked ground shows the mover.
pub const HEXLOCK_MSG: &str = "The hex grid flares crimson: that ground is contested. Movement there is \
     locked to the battle map.";

/// In-RAM working state (a [`Registrar::state`] slot). Everything that
/// must survive a reboot rides [`ModuleOp`](mush_core::ModuleOp)s instead.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TacticsState {
    /// Fires of the `pulse` fast tick since boot.
    pub pulses: u64,
    /// Active test battles: starter → scene.
    pub battles: HashMap<ObjId, SceneId>,
}

/// The stub module. Hand to
/// [`Engine::with_modules`](mush_core::engine::Engine::with_modules).
#[derive(Debug, Default)]
pub struct TacticsStub;

impl TacticsStub {
    /// A fresh stub.
    #[must_use]
    pub fn new() -> TacticsStub {
        TacticsStub
    }
}

impl GameModule for TacticsStub {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(TacticsState::default());
        reg.marker_flag(HEXLOCK_MARKER)?;
        reg.hook(HookPoint::MoveAttempt, Arc::new(HexLock));
        reg.hook(HookPoint::SceneEnded, Arc::new(SceneJanitor));
        // Scene owners MUST watch ObjDestroyed: the world silently drops a
        // destroyed husk from scene member sets, so without this a dead
        // starter would leak its battle (and its 250ms ticker) forever.
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(on_obj_destroyed));
        // The reserved WeatherQuery seam (checklist: environment queries).
        // The stub provides a fixed observation so consumers can exercise
        // the read path end to end; the real weather module (M5) replaces
        // this registration in production wiring.
        reg.weather_query(Arc::new(StubWeather))?;
        let feed = reg.panel_feed(MAP_FEED)?;

        // Checklist 1: a real sub-second cadence.
        reg.tick(
            PULSE_TICK,
            TickCadence::Fast(PULSE_PERIOD),
            Arc::new(|ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>| {
                if let Some(st) = ctx.state_mut::<TacticsState>() {
                    st.pulses += 1;
                }
            }),
        )?;

        // Checklist 2+3+4: the per-scene ticker mutating ruleset state and
        // streaming panel deltas.
        let tick_feed = feed.clone();
        reg.tick(
            BATTLE_TICK,
            TickCadence::Scene,
            Arc::new(
                move |ctx: &mut Ctx<'_>, _tag: &str, scene: Option<SceneId>| {
                    if let Some(scene) = scene {
                        battle_tick(&tick_feed, ctx, scene);
                    }
                },
            ),
        )?;

        let cmd_feed = feed;
        reg.command(
            CommandSpec {
                name: "+btest".into(),
                switches: vec!["start".into(), "stop".into(), "status".into()],
                shape: ArgShape::Raw,
                help: "+btest/start | /stop | /status  --  The tactical seam test. \
                       /start opens a toy battle scene around you: a 2x2 hex map blob \
                       is stored, a per-scene reactor tick (250ms) starts bumping your \
                       heat, and the tactics.map panel streams a keyframe then deltas. \
                       /status reports your heat and the module pulse count; /stop \
                       ends the scene (state and map are deleted from the store)."
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| btest_cmd(&cmd_feed, ctx)),
        )?;
        Ok(())
    }

    /// Purge stale scene documents (the scene boot contract — see
    /// [`mush_core::Scene`]): scene ids restart at 0 every boot, so any
    /// `scene:*` key that survived a crash-without-`end_scene` would
    /// interleave with this boot's scene #0. Queue a `Del` for each.
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        for (key, _) in data.kv {
            if key.starts_with("scene:") {
                ctx.persist(ModuleOp {
                    ns: SmolStr::new_static(NS),
                    op: ModuleWrite::Del { key },
                });
            }
        }
        Ok(())
    }
}

/// The stub weather provider: one fixed observation, enough to prove the
/// read path (`Registry::weather_query` → `current`) end to end.
struct StubWeather;

impl WeatherQuery for StubWeather {
    fn current(&self, _w: &World, _room: ObjId) -> Option<WxSnapshot> {
        Some(WxSnapshot {
            state: SmolStr::new_static("clear"),
            visibility: 1.0,
            precipitation: 0.0,
            wind: 2.0,
            temperature: 21.0,
        })
    }
}

/// `ObjDestroyed` observer: end any battle whose starter was destroyed and
/// scrub the husk from every battle scene it was merely a member of;
/// emptied scenes end too (their tickers die with them). Without this a
/// `@destroy`ed starter leaks a 250ms scene ticker until reboot.
fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    let obj = *obj;
    // Battles the husk STARTED end outright.
    let started: Vec<SceneId> = ctx
        .state::<TacticsState>()
        .map(|st| {
            st.battles
                .iter()
                .filter(|(starter, _)| **starter == obj)
                .map(|(_, s)| *s)
                .collect()
        })
        .unwrap_or_default();
    for scene in started {
        ctx.end_scene(scene); // SceneJanitor drops the battles entry
    }
    // Battles it merely fought in: the world already removed it from the
    // member set; end any battle scene that emptied out.
    let emptied: Vec<SceneId> = ctx
        .state::<TacticsState>()
        .map(|st| st.battles.values().copied().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|&s| {
            ctx.w
                .scene(s)
                .is_some_and(|sc| sc.kind == SCENE_KIND && sc.members.is_empty())
        })
        .collect();
    for scene in emptied {
        ctx.end_scene(scene);
    }
}

/// Checklist 5: the veto-capable `MoveAttempt` hook. `dest` covers exit
/// traversal and `@teleport` alike (`exit` is `None` for teleports).
struct HexLock;

impl EventHook for HexLock {
    fn move_attempt(
        &self,
        w: &mut World,
        _actor: ObjId,
        _exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), RichText> {
        if w.has_marker(dest, HEXLOCK_MARKER) {
            return Err(RichText::tag("error", HEXLOCK_MSG));
        }
        Ok(())
    }
}

/// Drops ended scenes from [`TacticsState::battles`] — however the scene
/// ended (also exercises the `SceneEnded` hook seam).
struct SceneJanitor;

impl EventHook for SceneJanitor {
    fn scene_ended(&self, ctx: &mut Ctx<'_>, scene: SceneId) {
        if let Some(st) = ctx.state_mut::<TacticsState>() {
            st.battles.retain(|_, s| *s != scene);
        }
    }
}

/// One battle tick: bump every member's heat in the per-scene state store
/// (each set persists a whole-scene snapshot), then delta the new heat to
/// each member's panel.
fn battle_tick(feed: &PanelFeedHandle, ctx: &mut Ctx<'_>, scene: SceneId) {
    let members: Vec<ObjId> = ctx
        .w
        .scene(scene)
        .map(|s| s.members.iter().copied().collect())
        .unwrap_or_default();
    let mut heats = serde_json::Map::new();
    for m in &members {
        let heat = ctx
            .scene_state(scene, *m)
            .and_then(|v| v.get("heat"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            + 1;
        let _ = ctx.scene_state_set(scene, *m, serde_json::json!({ "heat": heat }));
        heats.insert(m.to_string(), serde_json::json!(heat));
    }
    let data = serde_json::json!({ "scene": scene.0, "heat": heats });
    for m in members {
        feed.delta_to(ctx, Audience::Actor(m), data.clone());
    }
}

/// A toy 2x2 hex map blob — core stores and returns it, never reads it.
fn toy_map() -> serde_json::Value {
    serde_json::json!({
        "w": 2,
        "h": 2,
        "hexes": [
            { "q": 0, "r": 0, "terrain": "clear", "elev": 0 },
            { "q": 1, "r": 0, "terrain": "light-woods", "elev": 0 },
            { "q": 0, "r": 1, "terrain": "rough", "elev": 1 },
            { "q": 1, "r": 1, "terrain": "water", "elev": -1 },
        ],
    })
}

fn btest_cmd(feed: &PanelFeedHandle, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if ctx.switches.has("start") {
        let already = ctx
            .state::<TacticsState>()
            .is_some_and(|st| st.battles.contains_key(&actor));
        if already {
            return Err(CmdError::msg(
                "You are already in a test battle (+btest/stop).",
            ));
        }
        let scene = ctx.start_scene(SCENE_KIND, BTreeSet::from([actor]));
        ctx.set_map_blob(scene, toy_map())?;
        ctx.scene_state_set(scene, actor, serde_json::json!({ "heat": 0 }))?;
        ctx.scene_tick_start(scene, BATTLE_TICK, BATTLE_PERIOD)?;
        if let Some(st) = ctx.state_mut::<TacticsState>() {
            st.battles.insert(actor, scene);
        }
        feed.keyframe_to(
            ctx,
            Audience::Actor(actor),
            serde_json::json!({
                "scene": scene.0,
                "map": toy_map(),
                "units": { actor.to_string(): { "heat": 0 } },
            }),
        );
        ctx.out.info(
            actor,
            format!("BTEST: battle {scene} started; reactor ticking every 250ms."),
        );
        return Ok(());
    }
    if ctx.switches.has("stop") {
        let scene = ctx
            .state::<TacticsState>()
            .and_then(|st| st.battles.get(&actor).copied())
            .ok_or_else(|| CmdError::msg("You are not in a test battle."))?;
        ctx.end_scene(scene); // SceneJanitor drops the state entry
        ctx.out.info(
            actor,
            format!("BTEST: battle {scene} ended; map and state deleted."),
        );
        return Ok(());
    }
    if ctx.switches.has("status") {
        let (pulses, scene) = ctx
            .state::<TacticsState>()
            .map(|st| (st.pulses, st.battles.get(&actor).copied()))
            .unwrap_or((0, None));
        let battle = match scene {
            Some(s) => {
                let heat = ctx
                    .scene_state(s, actor)
                    .and_then(|v| v.get("heat"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                format!("{s}, heat {heat}")
            }
            None => "none".to_string(),
        };
        ctx.out
            .info(actor, format!("BTEST: pulses {pulses}; battle: {battle}."));
        return Ok(());
    }
    Err(CmdError::msg("Usage: +btest/start | /stop | /status"))
}
