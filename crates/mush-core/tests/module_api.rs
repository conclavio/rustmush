//! The module-API proving ground: one `ToyModule` exercising EVERY
//! [`Registrar`] surface end to end through the real [`Engine`] —
//! commands, alias stages, marker flags, attr-namespace enforcement,
//! coarse/fast/scene ticks, the MoveAttempt veto, LookRendered, world-event
//! observers, state slots, panel frames, persistence ops, scenes, and the
//! input-context stack. This suite is the M3 milestone's contract: if the
//! API can't express the toy, it can't express comsys/mail/tactics either.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::module::{
    ContextOutcome, EventHook, GameModule, HookPoint, InputContext, JobQueueSpec, Migration,
    ModError, ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, PanelFeedHandle, QueryHandler,
    RecoveryReporter, Registrar, SceneId, TickCadence, WorldEvent, WorldEventKind,
};
use mush_core::{
    ActionError, ActionInvocation, ActionProvider, AliasOutcome, AliasStage, ArgShape,
    AuthorityGuard, AuthorityGuardKind, AuthorityGuardValidator, BridgeDescriptor, CanonicalId,
    CmdError, CommandSpec, Ctx, DefinitionDescriptor, DefinitionDigest, DefinitionId,
    DefinitionKey, DefinitionKind, DefinitionRef, DefinitionValidationContext, GOD, IncidentId,
    IncidentState, LIMBO, MAX_BOOT_REPORT_DETAILS, MigrationPlanSummary, ModuleBootReport, ObjId,
    ObjKind, Outbox, OwnerId, RecoveryIncidentSummary, RecoveryReason, RecoveryScope,
    RecoverySeverity, Registry, RevisionKey, RevisionKind, RevisionValidator, SubjectKey, World,
    register_core_commands, register_engine_commands,
};
use mush_proto::{InputMode, NarrativeKind, OutputEvent, PanelFrameKind, PromptSpec, RichText};
use serde_json::json;
use smol_str::SmolStr;

// ----------------------------------------------------------------------
// The toy module
// ----------------------------------------------------------------------

/// Everything the toy records, living in a typed state slot on the World.
#[derive(Clone, Debug, Default)]
struct ToyState {
    bumps: u32,
    coarse_ticks: u32,
    fast_ticks: u32,
    scene_ticks: u32,
    whoami: Option<Option<SmolStr>>,
    moved: Vec<(ObjId, ObjId)>,
    said: Vec<String>,
    created: Vec<ObjId>,
    destroyed: Vec<ObjId>,
    ev_connected: Vec<ObjId>,
    ev_disconnected: Vec<ObjId>,
    hook_connected: Vec<ObjId>,
    hook_disconnected: Vec<ObjId>,
    scenes_started: Vec<u64>,
    scenes_ended: Vec<u64>,
    scene: Option<SceneId>,
    compose_lines: Vec<String>,
    boot_kv: Vec<(SmolStr, serde_json::Value)>,
    boot_stream_len: usize,
}

fn state<'a>(ctx: &'a mut Ctx<'_>) -> &'a mut ToyState {
    ctx.state_mut::<ToyState>().expect("toy state slot")
}

struct ToyHook;

impl EventHook for ToyHook {
    fn move_attempt(
        &self,
        w: &mut World,
        _actor: ObjId,
        _exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), RichText> {
        if w.get(dest).is_some_and(|o| o.name == "Vault") {
            return Err(RichText::plain("The wards flare."));
        }
        Ok(())
    }

    fn look_rendered(&self, _w: &World, _looker: ObjId, _room: ObjId) -> Vec<RichText> {
        vec![RichText::plain("A toy glows here.")]
    }

    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        state(ctx).hook_connected.push(actor);
    }

    fn disconnected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        state(ctx).hook_disconnected.push(actor);
    }

    fn scene_started(&self, ctx: &mut Ctx<'_>, scene: SceneId) {
        state(ctx).scenes_started.push(scene.0);
    }

    fn scene_ended(&self, ctx: &mut Ctx<'_>, scene: SceneId) {
        state(ctx).scenes_ended.push(scene.0);
    }
}

struct ToyAliases;

impl AliasStage for ToyAliases {
    fn try_alias(
        &self,
        _w: &mut World,
        actor: ObjId,
        line: &str,
        out: &mut Outbox,
    ) -> AliasOutcome {
        if let Some(rest) = line.strip_prefix("toyecho ") {
            out.info(actor, format!("echo: {rest}"));
            return AliasOutcome::Handled;
        }
        if let Some(rest) = line.strip_prefix("toyrw ") {
            return AliasOutcome::Rewrite(format!("say {rest}"));
        }
        // A malicious/classic alias shape: claims the verb "say" itself
        // (what `addcom say=Public` would set up). Prefix-expanded lines
        // must never reach it.
        if line == "say trapme" {
            out.info(actor, "alias captured say");
            return AliasOutcome::Handled;
        }
        AliasOutcome::NotMine
    }
}

struct ToyCompose;

impl InputContext for ToyCompose {
    fn prompt(&self, _w: &World) -> PromptSpec {
        PromptSpec {
            mode: InputMode::Normal,
            text: Some(RichText::plain("compose>")),
        }
    }

    fn feed(&mut self, ctx: &mut Ctx<'_>, line: &str) -> ContextOutcome {
        if line == "." {
            return ContextOutcome::Pop;
        }
        if let Some(rest) = line.strip_prefix("run ") {
            return ContextOutcome::PopAndRun(rest.to_string());
        }
        state(ctx).compose_lines.push(line.to_string());
        ContextOutcome::Consumed
    }
}

struct ToyQuery;

impl QueryHandler for ToyQuery {
    fn query(
        &self,
        _w: &World,
        actor: ObjId,
        params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static("toy.panel"),
            kind: PanelFrameKind::Keyframe,
            data: json!({ "actor": actor.to_string(), "echo": params }),
        })
    }
}

struct ToyModule;

impl GameModule for ToyModule {
    fn name(&self) -> &'static str {
        "toy"
    }

    #[allow(clippy::too_many_lines)]
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        // Typed state slot.
        reg.state(ToyState::default());

        // Markers, attr namespace, script api, job queue, migrations.
        reg.marker_flag("toy:shiny")?;
        reg.attr_namespace("toy")?;
        reg.script_api("toy")?;
        reg.job_queue(JobQueueSpec {
            name: "toyq".into(),
            title: "Toy requests".into(),
            ..JobQueueSpec::default()
        })?;
        reg.migrations(
            "toy",
            vec![Migration {
                version: 1,
                description: "add version field".into(),
                apply: Arc::new(|_key: &str, mut v| {
                    v["v"] = json!(1);
                    Ok(v)
                }),
            }],
        )?;

        // Query handler (registry only until M4).
        reg.query_handler("toy.list", Arc::new(ToyQuery))?;

        // Alias stage.
        reg.alias_stage(Arc::new(ToyAliases));

        // Hooks: veto + look + conn + scene lifecycle.
        for point in [
            HookPoint::MoveAttempt,
            HookPoint::LookRendered,
            HookPoint::Connected,
            HookPoint::Disconnected,
            HookPoint::SceneStarted,
            HookPoint::SceneEnded,
        ] {
            reg.hook(point, Arc::new(ToyHook));
        }

        // Observers.
        reg.subscribe(
            WorldEventKind::Moved,
            Arc::new(|ctx: &mut Ctx<'_>, ev: &WorldEvent| {
                if let WorldEvent::Moved { who, to, .. } = ev {
                    state(ctx).moved.push((*who, *to));
                }
            }),
        );
        reg.subscribe(
            WorldEventKind::Said {
                kind: NarrativeKind::Say,
            },
            Arc::new(|ctx: &mut Ctx<'_>, ev: &WorldEvent| {
                if let WorldEvent::Said { text, .. } = ev {
                    state(ctx).said.push(text.clone());
                }
            }),
        );
        reg.subscribe(
            WorldEventKind::ObjCreated,
            Arc::new(|ctx: &mut Ctx<'_>, ev: &WorldEvent| {
                if let WorldEvent::ObjCreated { obj, .. } = ev {
                    state(ctx).created.push(*obj);
                }
            }),
        );
        reg.subscribe(
            WorldEventKind::ObjDestroyed,
            Arc::new(|ctx: &mut Ctx<'_>, ev: &WorldEvent| {
                if let WorldEvent::ObjDestroyed { obj } = ev {
                    state(ctx).destroyed.push(*obj);
                }
            }),
        );
        reg.subscribe(
            WorldEventKind::Connected,
            Arc::new(|ctx: &mut Ctx<'_>, ev: &WorldEvent| {
                if let WorldEvent::Connected { who } = ev {
                    state(ctx).ev_connected.push(*who);
                }
            }),
        );
        reg.subscribe(
            WorldEventKind::Disconnected,
            Arc::new(|ctx: &mut Ctx<'_>, ev: &WorldEvent| {
                if let WorldEvent::Disconnected { who } = ev {
                    state(ctx).ev_disconnected.push(*who);
                }
            }),
        );

        // Ticks: coarse (5s), fast (sub-second!), and a scene ticker.
        reg.tick(
            "pulse",
            TickCadence::Coarse(Duration::from_secs(5)),
            Arc::new(|ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>| {
                state(ctx).coarse_ticks += 1;
            }),
        )?;
        reg.tick(
            "fast",
            TickCadence::Fast(Duration::from_millis(250)),
            Arc::new(|ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>| {
                state(ctx).fast_ticks += 1;
            }),
        )?;
        reg.tick(
            "battle",
            TickCadence::Scene,
            Arc::new(|ctx: &mut Ctx<'_>, _tag: &str, scene: Option<SceneId>| {
                assert!(scene.is_some(), "scene ticks carry their scene");
                state(ctx).scene_ticks += 1;
            }),
        )?;

        // Panel feed + commands.
        let panel: PanelFeedHandle = reg.panel_feed("toy.panel")?;
        reg.command(
            CommandSpec {
                name: "+panel".into(),
                shape: ArgShape::None,
                help: "+panel -- emit a toy panel keyframe.".into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                panel.keyframe(ctx, json!({"hello": 1}));
                Ok(())
            }),
        )?;
        reg.command(
            CommandSpec {
                name: "+bump".into(),
                shape: ArgShape::None,
                help: "+bump -- increment the toy counter.".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                state(ctx).bumps += 1;
                let n = ctx.state::<ToyState>().expect("slot").bumps;
                ctx.out.info(ctx.actor, format!("bumped to {n}"));
                Ok(())
            }),
        )?;
        reg.command(
            CommandSpec {
                name: "+whoami".into(),
                shape: ArgShape::None,
                help: "+whoami -- record the handler's module identity.".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let module = ctx.module.clone();
                state(ctx).whoami = Some(module);
                Ok(())
            }),
        )?;
        reg.command(
            CommandSpec {
                name: "+persist".into(),
                help: "+persist <key> -- queue a toy persistence op.".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let key = SmolStr::new(ctx.args.raw());
                ctx.persist(ModuleOp {
                    ns: "toy".into(),
                    op: ModuleWrite::JsonSet {
                        key,
                        value: json!({"ok": true}),
                    },
                });
                ctx.persist(ModuleOp {
                    ns: "toy".into(),
                    op: ModuleWrite::StreamAppend {
                        key: "history".into(),
                        value: json!("logged"),
                        maxlen: 100,
                    },
                });
                Ok(())
            }),
        )?;
        reg.command(
            CommandSpec {
                name: "+compose".into(),
                shape: ArgShape::None,
                help: "+compose -- enter the toy compose context.".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                ctx.push_input_context(Box::new(ToyCompose));
                Ok(())
            }),
        )?;
        reg.command(
            CommandSpec {
                name: "+failpersist".into(),
                shape: ArgShape::None,
                help: "+failpersist -- queue a persist op, then fail validation.".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                ctx.persist(ModuleOp {
                    ns: "toy".into(),
                    op: ModuleWrite::JsonSet {
                        key: "half-committed".into(),
                        value: json!(true),
                    },
                });
                Err(CmdError::msg("validation failed after persisting"))
            }),
        )?;
        reg.command(
            CommandSpec {
                name: "+scenestart".into(),
                shape: ArgShape::None,
                help: "+scenestart -- start a toy scene with a 100ms ticker.".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let actor = ctx.actor;
                let id = ctx.start_scene("toy.battle", BTreeSet::from([actor]));
                state(ctx).scene = Some(id);
                ctx.scene_tick_start(id, "battle", Duration::from_millis(100))?;
                ctx.scene_state_set(id, actor, json!({"hp": 10}))?;
                ctx.set_map_blob(id, json!({"hex": [0, 0]}))?;
                Ok(())
            }),
        )?;
        reg.command(
            CommandSpec {
                name: "+scenetickstop".into(),
                shape: ArgShape::None,
                help: "+scenetickstop -- stop the toy scene ticker.".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let id = ctx.state::<ToyState>().and_then(|s| s.scene);
                if let Some(id) = id {
                    ctx.scene_tick_stop(id, "battle");
                }
                Ok(())
            }),
        )?;
        reg.command(
            CommandSpec {
                name: "+sceneend".into(),
                shape: ArgShape::None,
                help: "+sceneend -- end the toy scene.".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let id = ctx.state::<ToyState>().and_then(|s| s.scene);
                if let Some(id) = id {
                    ctx.end_scene(id);
                }
                Ok(())
            }),
        )?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let stream_len = data.streams.get("history").map_or(0, Vec::len);
        let st = state(ctx);
        st.boot_kv = data.kv;
        st.boot_stream_len = stream_len;
        ctx.persist(ModuleOp {
            ns: "toy".into(),
            op: ModuleWrite::JsonSet {
                key: "booted".into(),
                value: json!(true),
            },
        });
        Ok(())
    }
}

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

/// Engine with core + engine commands and the toy module, now = 1000s,
/// plus player Nick in Limbo.
fn setup() -> (Engine, ObjId) {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let engine = Engine::with_modules(world, registry, vec![Box::new(ToyModule)]).unwrap();
    (engine, nick)
}

/// Run one command line through the engine, returning its outbox.
fn run(engine: &mut Engine, actor: ObjId, line: &str) -> Outbox {
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::Cmd {
            conn: 1,
            actor,
            line: line.into(),
        },
        &mut out,
        &mut fx,
    );
    out
}

fn toy(engine: &Engine) -> &ToyState {
    engine.world.module_state::<ToyState>().expect("toy state")
}

fn sys_texts(out: &Outbox) -> Vec<String> {
    out.events()
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Sys(t) => Some(t.to_plain_text()),
            _ => None,
        })
        .collect()
}

fn prompts(out: &Outbox) -> Vec<PromptSpec> {
    out.events()
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Prompt(p) => Some(p.clone()),
            _ => None,
        })
        .collect()
}

fn fire(engine: &mut Engine, now_ms: i64) -> Outbox {
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.set_now(now_ms / 1000);
    engine.fire_due(now_ms, &mut out, &mut fx);
    out
}

// ----------------------------------------------------------------------
// Registrar surfaces
// ----------------------------------------------------------------------

#[test]
fn registrar_populates_every_table() {
    let (engine, _) = setup();
    let reg = &engine.registry;
    // Commands are in the ONE registry, module-tagged.
    assert!(reg.spec_of("+panel").is_some());
    assert!(reg.spec_of("+scenestart").is_some());
    // Marker flags, attr namespaces (case-insensitive).
    let (canonical, owner) = reg.marker_owner("TOY:SHINY").expect("marker registered");
    assert_eq!((canonical.as_str(), owner.as_str()), ("toy:shiny", "toy"));
    assert_eq!(reg.attr_ns_owner("Toy").map(SmolStr::as_str), Some("toy"));
    assert_eq!(reg.attr_ns_owner("other"), None);
    // Ticks.
    assert_eq!(reg.module_ticks().len(), 3);
    assert!(matches!(
        reg.tick_reg("toy", "battle").map(|t| t.cadence),
        Some(TickCadence::Scene)
    ));
    // Panel feeds, query handlers, job queues, script apis, migrations.
    assert_eq!(
        reg.panel_feeds(),
        &[(SmolStr::new("toy.panel"), SmolStr::new("toy"))]
    );
    let (owner, q) = reg.query_handler("toy.list").expect("query handler");
    assert_eq!(owner.as_str(), "toy");
    let ans = q.query(&engine.world, GOD, &json!({"n": 3})).unwrap();
    let OutputEvent::Panel { data, .. } = ans else {
        panic!("expected Panel, got {ans:?}");
    };
    assert_eq!(data["echo"]["n"], json!(3));
    assert_eq!(reg.job_queues()[0].name, "toyq");
    assert_eq!(reg.script_apis()[0].0, "toy");
    let ms = reg.migrations().get("toy").expect("migrations collected");
    assert_eq!(ms[0].version, 1);
    assert_eq!(
        (ms[0].apply)("doc:1", json!({"a": 1})).unwrap(),
        json!({"a": 1, "v": 1})
    );
}

#[test]
fn registrar_rejects_cross_module_conflicts() {
    let mut reg = Registry::new();
    let mut one = Registrar::new(&mut reg, "one");
    one.attr_namespace("dup").unwrap();
    one.marker_flag("dup:mark").unwrap();
    one.panel_feed("dup.feed").unwrap();
    one.script_api("dup").unwrap();
    one.query_handler("dup.q", Arc::new(ToyQuery)).unwrap();
    one.job_queue(JobQueueSpec {
        name: "dupq".into(),
        ..JobQueueSpec::default()
    })
    .unwrap();
    one.tick(
        "t",
        TickCadence::Scene,
        Arc::new(|_: &mut Ctx<'_>, _: &str, _: Option<SceneId>| {}),
    )
    .unwrap();

    let mut two = Registrar::new(&mut reg, "two");
    assert!(matches!(
        two.attr_namespace("DUP"),
        Err(ModError::Conflict {
            what: "attr namespace",
            ..
        })
    ));
    assert!(matches!(
        two.marker_flag("DUP:MARK"),
        Err(ModError::Conflict {
            what: "marker flag",
            ..
        })
    ));
    assert!(matches!(
        two.panel_feed("dup.feed"),
        Err(ModError::Conflict {
            what: "panel feed",
            ..
        })
    ));
    assert!(matches!(
        two.script_api("dup"),
        Err(ModError::Conflict {
            what: "script api",
            ..
        })
    ));
    assert!(matches!(
        two.query_handler("dup.q", Arc::new(ToyQuery)),
        Err(ModError::Conflict {
            what: "query handler",
            ..
        })
    ));
    assert!(matches!(
        two.job_queue(JobQueueSpec {
            name: "dupq".into(),
            ..JobQueueSpec::default()
        }),
        Err(ModError::Conflict {
            what: "job queue",
            ..
        })
    ));
    // Tick tags are per-module: module two may reuse "t"...
    two.tick(
        "t",
        TickCadence::Scene,
        Arc::new(|_: &mut Ctx<'_>, _: &str, _: Option<SceneId>| {}),
    )
    .unwrap();
    // ...but not twice.
    assert!(matches!(
        two.tick(
            "t",
            TickCadence::Scene,
            Arc::new(|_: &mut Ctx<'_>, _: &str, _: Option<SceneId>| {}),
        ),
        Err(ModError::Conflict {
            what: "tick tag",
            ..
        })
    ));
    // A colliding command name is a registry error, wrapped.
    let mut three = Registrar::new(&mut reg, "three");
    three
        .command(
            CommandSpec {
                name: "+dup".into(),
                ..CommandSpec::default()
            },
            Arc::new(|_: &mut Ctx<'_>| Ok(())),
        )
        .unwrap();
    assert!(matches!(
        three.command(
            CommandSpec {
                name: "+dup".into(),
                ..CommandSpec::default()
            },
            Arc::new(|_: &mut Ctx<'_>| Ok(())),
        ),
        Err(ModError::Registry(_))
    ));
}

/// The migration-namespace claim: the first registrant owns it; a second
/// module extending the same namespace is refused at registration (two
/// merged lists would fail the boot-time ascending check confusingly).
#[test]
fn migration_namespace_is_claimed_by_first_registrant() {
    let mut reg = Registry::new();
    let mig = || {
        vec![Migration {
            version: 1,
            description: "x".into(),
            apply: Arc::new(|_k: &str, v| Ok(v)),
        }]
    };
    let mut one = Registrar::new(&mut reg, "one");
    one.migrations("shared", mig()).unwrap();
    // The owner may append more steps.
    one.migrations(
        "shared",
        vec![Migration {
            version: 2,
            description: "y".into(),
            apply: Arc::new(|_k: &str, v| Ok(v)),
        }],
    )
    .unwrap();
    let mut two = Registrar::new(&mut reg, "two");
    assert!(matches!(
        two.migrations("shared", mig()),
        Err(ModError::Conflict {
            what: "migration namespace",
            ..
        })
    ));
}

/// The weather seam: single provider, second registration refused,
/// readable through the registry.
#[test]
fn weather_query_seam_registers_and_conflicts() {
    use mush_core::{WeatherQuery, WxSnapshot};
    struct Wx;
    impl WeatherQuery for Wx {
        fn current(&self, _w: &World, _room: ObjId) -> Option<WxSnapshot> {
            Some(WxSnapshot {
                state: "rain".into(),
                visibility: 0.5,
                precipitation: 0.8,
                wind: 10.0,
                temperature: 12.0,
            })
        }
    }
    let mut reg = Registry::new();
    let mut one = Registrar::new(&mut reg, "weather");
    one.weather_query(Arc::new(Wx)).unwrap();
    let mut two = Registrar::new(&mut reg, "tactics");
    assert!(matches!(
        two.weather_query(Arc::new(Wx)),
        Err(ModError::Conflict {
            what: "weather provider",
            ..
        })
    ));
    let (owner, provider) = reg.weather_query().expect("provider registered");
    assert_eq!(owner.as_str(), "weather");
    let w = World::new_minimal();
    let wx = provider.current(&w, LIMBO).expect("snapshot");
    assert_eq!(wx.state, "rain");
}

/// State slots collide loudly, not silently: a second module declaring the
/// same slot type is a boot error (it would wipe the first module's slot —
/// e.g. every registered jobs payload handler).
#[test]
fn duplicate_state_slot_across_modules_fails_boot() {
    struct A;
    impl GameModule for A {
        fn name(&self) -> &'static str {
            "a"
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.state(ToyState::default());
            Ok(())
        }
    }
    struct B;
    impl GameModule for B {
        fn name(&self) -> &'static str {
            "b"
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.state(ToyState::default()); // same TypeId as A's
            Ok(())
        }
    }
    let err = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(A), Box::new(B)],
    )
    .err()
    .expect("duplicate slot type must fail boot");
    assert!(err.to_string().contains("state slot"), "{err}");

    // A module re-declaring its OWN slot is fine (last value wins).
    struct C;
    impl GameModule for C {
        fn name(&self) -> &'static str {
            "c"
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.state(ToyState::default());
            reg.state(ToyState {
                bumps: 7,
                ..ToyState::default()
            });
            Ok(())
        }
    }
    let engine =
        Engine::with_modules(World::new_minimal(), Registry::new(), vec![Box::new(C)]).unwrap();
    assert_eq!(engine.world.module_state::<ToyState>().unwrap().bumps, 7);
}

/// Duplicate module names would merge persistence namespaces and clobber
/// boot data — refused at construction.
#[test]
fn duplicate_module_names_fail_boot() {
    struct M;
    impl GameModule for M {
        fn name(&self) -> &'static str {
            "twin"
        }
        fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), ModError> {
            Ok(())
        }
    }
    let err = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(M), Box::new(M)],
    )
    .err()
    .expect("duplicate module names must fail boot");
    assert!(err.to_string().contains("twin"), "{err}");
}

#[test]
fn mechanics_owner_roster_is_bounded_at_composition() {
    struct EmptyOwner(&'static str);
    impl GameModule for EmptyOwner {
        fn name(&self) -> &'static str {
            self.0
        }

        fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), ModError> {
            Ok(())
        }
    }

    let modules: Vec<Box<dyn GameModule>> = (0..=mush_core::MAX_MECHANICS_OWNERS)
        .map(|index| {
            let name: &'static str = Box::leak(format!("owner_{index}").into_boxed_str());
            Box::new(EmptyOwner(name)) as Box<dyn GameModule>
        })
        .collect();
    let error = Engine::compose_modules(Registry::new(), modules)
        .err()
        .expect("oversized mechanics owner roster must fail composition");
    assert!(
        error.to_string().contains("mechanics owner roster"),
        "{error}"
    );
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PreWorldState(u32);

struct MechanicsDefinitionModule;

struct AcceptAction;

impl ActionProvider for AcceptAction {
    fn validate_invocation(&self, _invocation: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
}

struct MechanicsActionModule {
    name: &'static str,
    duplicate_provider: bool,
}

struct ModuleRevisionValidator;

impl RevisionValidator for ModuleRevisionValidator {
    fn current_generation(
        &self,
        _world: &World,
        _key: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        Ok(Some(1))
    }
}

struct ModuleAuthorityValidator;

impl AuthorityGuardValidator for ModuleAuthorityValidator {
    fn is_current(&self, _world: &World, _guard: &AuthorityGuard) -> Result<bool, ActionError> {
        Ok(true)
    }
}

struct MechanicsGuardModule {
    duplicate_revision: bool,
}

impl GameModule for MechanicsGuardModule {
    fn name(&self) -> &'static str {
        "guard_test"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let kind = RevisionKind::new(CanonicalId::new("unit").unwrap());
        reg.revision_validator(kind.clone(), ModuleRevisionValidator)?;
        if self.duplicate_revision {
            reg.revision_validator(kind, ModuleRevisionValidator)?;
        }
        reg.authority_guard_validator(AuthorityGuardKind::Capability, ModuleAuthorityValidator)
    }
}

impl GameModule for MechanicsActionModule {
    fn name(&self) -> &'static str {
        self.name
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let kind = DefinitionKind::new("action").unwrap();
        reg.definition_validator(kind.clone(), object_definition_validator)?;
        let action = reg.definition(
            DefinitionDescriptor::new(
                kind,
                DefinitionId::new("wave").unwrap(),
                1,
                json!({"parameters": {}}),
            )
            .unwrap()
            .as_current(),
        )?;
        reg.action_provider(action.clone(), AcceptAction)?;
        if self.duplicate_provider {
            reg.action_provider(action, AcceptAction)?;
        }
        Ok(())
    }
}

fn object_definition_validator(context: &DefinitionValidationContext<'_>) -> Result<(), String> {
    context
        .definition()
        .payload()
        .is_object()
        .then_some(())
        .ok_or_else(|| "test definition payload must be an object".into())
}

impl GameModule for MechanicsDefinitionModule {
    fn name(&self) -> &'static str {
        "mechanics_test"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(PreWorldState(17));
        reg.definition_validator(
            DefinitionKind::new("schema").unwrap(),
            object_definition_validator,
        )?;
        reg.definition(
            DefinitionDescriptor::new(
                DefinitionKind::new("schema").unwrap(),
                DefinitionId::new("unit").unwrap(),
                1,
                json!({"shape": "test"}),
            )
            .unwrap()
            .as_current(),
        )?;
        Ok(())
    }
}

#[test]
fn composition_freezes_mechanics_before_it_touches_a_world() {
    let composition =
        Engine::compose_modules(Registry::new(), vec![Box::new(MechanicsDefinitionModule)])
            .unwrap();

    let definitions = composition
        .registry()
        .definitions()
        .expect("definition registry is frozen by composition");
    assert!(composition.registry().bridges().is_some());
    assert!(composition.registry().definition_migrations().is_some());
    assert!(composition.registry().mechanics_fingerprint().is_some());
    assert!(definitions.is_fully_provider_validated());
    let key = DefinitionKey::new(
        OwnerId::new("mechanics_test").unwrap(),
        DefinitionKind::new("schema").unwrap(),
        DefinitionId::new("unit").unwrap(),
    );
    assert!(definitions.current(&key).is_some());

    let untouched = World::new_minimal();
    assert!(untouched.module_state::<PreWorldState>().is_none());
    let engine = Engine::from_composition(untouched, composition);
    assert_eq!(
        engine.world.module_state::<PreWorldState>(),
        Some(&PreWorldState(17))
    );
}

#[test]
fn composition_freezes_exact_owner_stamped_action_providers() {
    let composition = Engine::compose_modules(
        Registry::new(),
        vec![Box::new(MechanicsActionModule {
            name: "action_test",
            duplicate_provider: false,
        })],
    )
    .unwrap();
    let key = DefinitionKey::new(
        OwnerId::new("action_test").unwrap(),
        DefinitionKind::new("action").unwrap(),
        DefinitionId::new("wave").unwrap(),
    );
    let action = composition
        .registry()
        .definitions()
        .unwrap()
        .current(&key)
        .unwrap();
    let entry = composition
        .registry()
        .action_providers()
        .expect("action provider registry is frozen by composition")
        .get_exact(action)
        .unwrap();
    assert_eq!(entry.owner(), &OwnerId::new("action_test").unwrap());

    let duplicate = Engine::compose_modules(
        Registry::new(),
        vec![Box::new(MechanicsActionModule {
            name: "action_test",
            duplicate_provider: true,
        })],
    )
    .err()
    .expect("an exact action provider can only be registered once");
    assert!(
        duplicate.to_string().contains("more than once"),
        "{duplicate}"
    );
}

#[test]
fn composed_owner_freeze_reaches_every_executable_registry_and_not_other_owners() {
    let composition = Engine::compose_modules(
        Registry::new(),
        vec![
            Box::new(MechanicsActionModule {
                name: "action_test",
                duplicate_provider: false,
            }),
            Box::new(MechanicsActionModule {
                name: "action_other",
                duplicate_provider: false,
            }),
            Box::new(MechanicsGuardModule {
                duplicate_revision: false,
            }),
        ],
    )
    .unwrap();
    let registry = composition.registry();
    let action_ref = |owner: &str| {
        registry
            .definitions()
            .unwrap()
            .current(&DefinitionKey::new(
                OwnerId::new(owner).unwrap(),
                DefinitionKind::new("action").unwrap(),
                DefinitionId::new("wave").unwrap(),
            ))
            .unwrap()
            .clone()
    };
    let held_action = action_ref("action_test");
    let other_action = action_ref("action_other");
    let held_owner = OwnerId::new("action_test").unwrap();
    let first_reason = RecoveryReason::IntegrityViolation {
        code: CanonicalId::new("persisted_definition_digest_conflict").unwrap(),
    };
    registry
        .freeze_mechanics_owner(&held_owner, IncidentId(501), first_reason)
        .unwrap();

    assert!(matches!(
        registry.definitions().unwrap().get_exact(&held_action),
        Err(mush_core::DefinitionLookupError::OwnerNotReady { .. })
    ));
    assert!(
        registry
            .definitions()
            .unwrap()
            .inspect_exact(&held_action)
            .is_ok()
    );
    assert!(matches!(
        registry.action_providers().unwrap().get_exact(&held_action),
        Err(mush_core::ActionProviderLookupError::OwnerNotReady { .. })
    ));
    assert!(
        registry
            .action_providers()
            .unwrap()
            .get_exact(&other_action)
            .is_ok()
    );
    assert!(
        registry
            .definitions()
            .unwrap()
            .get_exact(&other_action)
            .is_ok()
    );

    let guard_owner = OwnerId::new("guard_test").unwrap();
    let revision_key = RevisionKey {
        owner: guard_owner.clone(),
        kind: RevisionKind::new(CanonicalId::new("unit").unwrap()),
        subject: SubjectKey::Object { id: ObjId(17) },
        facet: None,
    };
    let authority_guard = AuthorityGuard::Capability {
        provider: guard_owner.clone(),
        actor: ObjId(17),
        capability: CanonicalId::new("pilot").unwrap(),
        generation: 1,
    };
    assert!(
        registry
            .revision_validators()
            .unwrap()
            .get_for_key(&revision_key)
            .is_ok()
    );
    assert!(
        registry
            .authority_guard_validators()
            .unwrap()
            .get_for_guard(&authority_guard)
            .is_ok()
    );
    registry
        .freeze_mechanics_owner(
            &guard_owner,
            IncidentId(502),
            RecoveryReason::IntegrityViolation {
                code: CanonicalId::new("guard_owner_conflict").unwrap(),
            },
        )
        .unwrap();
    assert!(matches!(
        registry
            .revision_validators()
            .unwrap()
            .get_for_key(&revision_key),
        Err(mush_core::RevisionValidatorLookupError::OwnerNotReady { .. })
    ));
    assert!(matches!(
        registry
            .authority_guard_validators()
            .unwrap()
            .get_for_guard(&authority_guard),
        Err(mush_core::AuthorityGuardValidatorLookupError::OwnerNotReady { .. })
    ));
    assert!(
        registry
            .action_providers()
            .unwrap()
            .get_exact(&other_action)
            .is_ok()
    );
}

#[test]
fn composition_freezes_owner_stamped_guard_validators() {
    let composition = Engine::compose_modules(
        Registry::new(),
        vec![Box::new(MechanicsGuardModule {
            duplicate_revision: false,
        })],
    )
    .unwrap();
    let key = RevisionKey {
        owner: OwnerId::new("guard_test").unwrap(),
        kind: RevisionKind::new(CanonicalId::new("unit").unwrap()),
        subject: SubjectKey::Object { id: ObjId(7) },
        facet: None,
    };
    let revision = composition
        .registry()
        .revision_validators()
        .expect("revision validators freeze during composition")
        .get_for_key(&key)
        .unwrap();
    assert_eq!(revision.owner(), &OwnerId::new("guard_test").unwrap());
    assert!(
        composition
            .registry()
            .authority_guard_validators()
            .is_some()
    );

    let duplicate = Engine::compose_modules(
        Registry::new(),
        vec![Box::new(MechanicsGuardModule {
            duplicate_revision: true,
        })],
    )
    .err()
    .expect("duplicate owner/kind revision validators must fail composition");
    assert!(
        duplicate.to_string().contains("more than once"),
        "{duplicate}"
    );
}

#[test]
fn standalone_registrar_cannot_preclaim_a_composed_mechanics_owner() {
    let mut registry = Registry::new();
    let descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("schema").unwrap(),
        DefinitionId::new("unit").unwrap(),
        1,
        json!({"shape": "spoofed"}),
    )
    .unwrap();

    let error = Registrar::new(&mut registry, "mechanics_test")
        .definition(descriptor)
        .expect_err("a standalone registrar must not carry mechanics authority");
    assert!(
        error
            .to_string()
            .contains("requires the Engine::compose_modules callback"),
        "{error}"
    );

    // The rejected attempt left no hidden builder entry, so the real module
    // can still claim and validate its own owner during composition.
    Engine::compose_modules(registry, vec![Box::new(MechanicsDefinitionModule)])
        .expect("composition-owned registrar remains authorized");
}

struct InvalidActionProviderModule {
    action: DefinitionRef,
}

impl GameModule for InvalidActionProviderModule {
    fn name(&self) -> &'static str {
        "action_test"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.action_provider(self.action.clone(), AcceptAction)
    }
}

#[test]
fn registrar_refuses_spoofed_or_unregistered_action_definitions() {
    let spoofed = DefinitionRef::new(
        DefinitionKey::new(
            OwnerId::new("other_owner").unwrap(),
            DefinitionKind::new("action").unwrap(),
            DefinitionId::new("wave").unwrap(),
        ),
        1,
        DefinitionDigest::from_bytes([1; 32]),
    )
    .unwrap();
    let error = Engine::compose_modules(
        Registry::new(),
        vec![Box::new(InvalidActionProviderModule { action: spoofed })],
    )
    .err()
    .expect("registrar owner stamping must reject spoofed action ownership");
    assert!(error.to_string().contains("not an action owned"), "{error}");

    let missing = DefinitionRef::new(
        DefinitionKey::new(
            OwnerId::new("action_test").unwrap(),
            DefinitionKind::new("action").unwrap(),
            DefinitionId::new("missing").unwrap(),
        ),
        1,
        DefinitionDigest::from_bytes([2; 32]),
    )
    .unwrap();
    let error = Engine::compose_modules(
        Registry::new(),
        vec![Box::new(InvalidActionProviderModule { action: missing })],
    )
    .err()
    .expect("provider freeze must require an exact registered definition");
    assert!(
        error.to_string().contains("exact definition") && error.to_string().contains("unavailable"),
        "{error}"
    );
}

struct MissingDefinitionValidatorModule;

impl GameModule for MissingDefinitionValidatorModule {
    fn name(&self) -> &'static str {
        "missing_validator"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.definition(
            DefinitionDescriptor::new(
                DefinitionKind::new("schema").unwrap(),
                DefinitionId::new("unchecked").unwrap(),
                1,
                json!({"shape": "unchecked"}),
            )
            .unwrap(),
        )?;
        Ok(())
    }
}

struct RejectedDefinitionModule;

impl GameModule for RejectedDefinitionModule {
    fn name(&self) -> &'static str {
        "rejected_definition"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.definition_validator(
            DefinitionKind::new("schema").unwrap(),
            object_definition_validator,
        )?;
        reg.definition(
            DefinitionDescriptor::new(
                DefinitionKind::new("schema").unwrap(),
                DefinitionId::new("not_an_object").unwrap(),
                1,
                json!(["canonical", "but", "semantically", "invalid"]),
            )
            .unwrap(),
        )?;
        Ok(())
    }
}

struct DuplicateDefinitionValidatorModule;

impl GameModule for DuplicateDefinitionValidatorModule {
    fn name(&self) -> &'static str {
        "duplicate_validator"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let kind = DefinitionKind::new("schema").unwrap();
        reg.definition_validator(kind.clone(), object_definition_validator)?;
        reg.definition_validator(kind, object_definition_validator)
    }
}

#[test]
fn registrar_requires_exactly_one_successful_validator_for_every_definition_kind() {
    let missing = Engine::compose_modules(
        Registry::new(),
        vec![Box::new(MissingDefinitionValidatorModule)],
    )
    .err()
    .expect("provider definitions without semantic validation must fail composition");
    assert!(
        missing.to_string().contains("did not register"),
        "{missing}"
    );

    let rejected =
        Engine::compose_modules(Registry::new(), vec![Box::new(RejectedDefinitionModule)])
            .err()
            .expect("canonical JSON rejected by the typed provider must fail composition");
    assert!(
        rejected
            .to_string()
            .contains("failed provider semantic validation"),
        "{rejected}"
    );

    let duplicate = Engine::compose_modules(
        Registry::new(),
        vec![Box::new(DuplicateDefinitionValidatorModule)],
    )
    .err()
    .expect("a second validator for the same owner and kind must fail composition");
    assert!(
        duplicate.to_string().contains("registered more than once"),
        "{duplicate}"
    );
}

struct RepeatedDefinitionModule {
    name: &'static str,
    conflict: bool,
}

impl GameModule for RepeatedDefinitionModule {
    fn name(&self) -> &'static str {
        self.name
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.definition_validator(
            DefinitionKind::new("schema").unwrap(),
            object_definition_validator,
        )?;
        let descriptor = |marker| {
            DefinitionDescriptor::new(
                DefinitionKind::new("schema").unwrap(),
                DefinitionId::new("unit").unwrap(),
                1,
                json!({"marker": marker}),
            )
            .unwrap()
        };
        reg.definition(descriptor(1))?;
        reg.definition(descriptor(if self.conflict { 2 } else { 1 }))?;
        Ok(())
    }
}

#[test]
fn registrar_definition_duplicates_and_digest_conflicts_fail_composition() {
    let duplicate = Engine::compose_modules(
        Registry::new(),
        vec![Box::new(RepeatedDefinitionModule {
            name: "duplicate_definition",
            conflict: false,
        })],
    )
    .err()
    .expect("an exact duplicate must fail");
    assert!(
        duplicate.to_string().contains("more than once"),
        "{duplicate}"
    );

    let conflict = Engine::compose_modules(
        Registry::new(),
        vec![Box::new(RepeatedDefinitionModule {
            name: "conflicting_definition",
            conflict: true,
        })],
    )
    .err()
    .expect("same-version digest drift must fail");
    assert!(
        conflict.to_string().contains("changes digest"),
        "{conflict}"
    );
}

struct MissingDefinitionDependency;

impl GameModule for MissingDefinitionDependency {
    fn name(&self) -> &'static str {
        "missing_dependency"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.definition_validator(
            DefinitionKind::new("schema").unwrap(),
            object_definition_validator,
        )?;
        let missing = DefinitionRef::new(
            DefinitionKey::new(
                OwnerId::new("absent").unwrap(),
                DefinitionKind::new("schema").unwrap(),
                DefinitionId::new("dependency").unwrap(),
            ),
            1,
            DefinitionDigest::from_bytes([9; 32]),
        )
        .unwrap();
        reg.definition(
            DefinitionDescriptor::new(
                DefinitionKind::new("schema").unwrap(),
                DefinitionId::new("dependent").unwrap(),
                1,
                json!({}),
            )
            .unwrap()
            .with_references(vec![missing]),
        )?;
        Ok(())
    }
}

#[test]
fn unresolved_exact_definition_reference_fails_the_freeze() {
    let error =
        Engine::compose_modules(Registry::new(), vec![Box::new(MissingDefinitionDependency)])
            .err()
            .expect("freeze must refuse an unavailable exact dependency");
    assert!(
        error.to_string().contains("missing exact definition"),
        "{error}"
    );
}

struct BridgeRosterModule {
    source: &'static str,
    target: &'static str,
}

impl GameModule for BridgeRosterModule {
    fn name(&self) -> &'static str {
        "bridge_provider"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.definition_validator(
            DefinitionKind::new("interaction").unwrap(),
            object_definition_validator,
        )?;
        reg.definition_validator(
            DefinitionKind::new("bridge").unwrap(),
            object_definition_validator,
        )?;
        let interaction = reg.definition(
            DefinitionDescriptor::new(
                DefinitionKind::new("interaction").unwrap(),
                DefinitionId::new("ram").unwrap(),
                1,
                json!({}),
            )
            .unwrap(),
        )?;
        let bridge = reg.definition(
            DefinitionDescriptor::new(
                DefinitionKind::new("bridge").unwrap(),
                DefinitionId::new("ram_bridge").unwrap(),
                1,
                json!({}),
            )
            .unwrap(),
        )?;
        reg.interaction_bridge(BridgeDescriptor::new(
            bridge,
            OwnerId::new(self.source).unwrap(),
            OwnerId::new(self.target).unwrap(),
            interaction,
        ))
    }
}

struct EmptyRosterModule(&'static str);

impl GameModule for EmptyRosterModule {
    fn name(&self) -> &'static str {
        self.0
    }

    fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), ModError> {
        Ok(())
    }
}

#[test]
fn bridge_endpoints_must_be_present_in_the_composed_module_roster() {
    for (source, target, endpoint, missing) in [
        (
            "absent_source",
            "bridge_provider",
            "source",
            "absent_source",
        ),
        (
            "bridge_provider",
            "absent_target",
            "target",
            "absent_target",
        ),
    ] {
        let error = Engine::compose_modules(
            Registry::new(),
            vec![Box::new(BridgeRosterModule { source, target })],
        )
        .err()
        .expect("a bridge endpoint cannot name an unloaded module");
        let message = error.to_string();
        assert!(message.contains(endpoint), "{message}");
        assert!(message.contains(missing), "{message}");
        assert!(message.contains("not present"), "{message}");
    }

    Engine::compose_modules(
        Registry::new(),
        vec![
            Box::new(BridgeRosterModule {
                source: "bridge_provider",
                target: "bridge_target",
            }),
            Box::new(EmptyRosterModule("bridge_target")),
        ],
    )
    .expect("a bridge whose endpoints are loaded is valid");
}

struct NamedModule {
    name: &'static str,
    registrations: Arc<std::sync::atomic::AtomicUsize>,
}

impl GameModule for NamedModule {
    fn name(&self) -> &'static str {
        self.name
    }

    fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), ModError> {
        self.registrations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn module_owner_names_are_canonical_unique_and_preflighted() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let error = Engine::compose_modules(
        Registry::new(),
        vec![
            Box::new(NamedModule {
                name: "Shared_Owner",
                registrations: Arc::clone(&calls),
            }),
            Box::new(NamedModule {
                name: "shared_owner",
                registrations: Arc::clone(&calls),
            }),
        ],
    )
    .err()
    .expect("canonical owner collision must fail");
    assert!(
        error.to_string().contains("same canonical mechanics owner"),
        "{error}"
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);

    let error = Engine::compose_modules(
        Registry::new(),
        vec![Box::new(NamedModule {
            name: "not-canonical",
            registrations: Arc::clone(&calls),
        })],
    )
    .err()
    .expect("invalid owner spelling must fail");
    assert!(
        error.to_string().contains("not a valid mechanics owner id"),
        "{error}"
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[derive(Clone)]
struct FixedRecoveryReport(ModuleBootReport);

impl RecoveryReporter for FixedRecoveryReport {
    fn report(&self, _world: &World) -> ModuleBootReport {
        self.0.clone()
    }
}

struct RecoveryModule {
    name: &'static str,
    report: ModuleBootReport,
}

impl GameModule for RecoveryModule {
    fn name(&self) -> &'static str {
        self.name
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.recovery_reporter(Arc::new(FixedRecoveryReport(self.report.clone())))
    }
}

fn test_definition_ref(owner: &str, byte: u8) -> DefinitionRef {
    DefinitionRef::new(
        DefinitionKey::new(
            OwnerId::new(owner).unwrap(),
            DefinitionKind::new("schema").unwrap(),
            DefinitionId::new("recovery").unwrap(),
        ),
        1,
        DefinitionDigest::from_bytes([byte; 32]),
    )
    .unwrap()
}

fn oversized_recovery_report() -> ModuleBootReport {
    let spoofed = OwnerId::new("spoofed").unwrap();
    let from = test_definition_ref("alpha", 1);
    let to = test_definition_ref("alpha", 2);
    let incidents = (0..=MAX_BOOT_REPORT_DETAILS)
        .map(|index| RecoveryIncidentSummary {
            id: IncidentId(index as u64),
            owner: spoofed.clone(),
            scope: RecoveryScope::Record,
            severity: RecoverySeverity::Error,
            reason: RecoveryReason::MalformedRecord {
                code: CanonicalId::new("bad_record").unwrap(),
            },
            subject_count: 0,
            payload_digest: None,
            discovered_unix: 0,
            state: IncidentState::Open,
        })
        .collect();
    let migration_plans = (0..=MAX_BOOT_REPORT_DETAILS)
        .map(|_| MigrationPlanSummary {
            from: from.clone(),
            to: to.clone(),
            step_count: 1,
            before_digest: DefinitionDigest::from_bytes([3; 32]),
            after_digest: DefinitionDigest::from_bytes([4; 32]),
        })
        .collect();
    ModuleBootReport {
        owner: spoofed,
        active: 1,
        frozen: 2,
        migrating: 3,
        quarantined: 4,
        incident_count: 0,
        migration_plan_count: 0,
        incidents,
        migration_plans,
        details_truncated: false,
    }
}

#[test]
fn recovery_reports_are_owner_stamped_bounded_and_canonically_ordered() {
    let alpha = oversized_recovery_report();
    let zeta = ModuleBootReport::new(OwnerId::new("spoofed").unwrap());
    let composition = Engine::compose_modules(
        Registry::new(),
        vec![
            Box::new(RecoveryModule {
                name: "zeta",
                report: zeta,
            }),
            Box::new(RecoveryModule {
                name: "alpha",
                report: alpha,
            }),
        ],
    )
    .unwrap();
    let reports = composition
        .registry()
        .recovery_reports(&World::new_minimal());
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].owner.as_str(), "alpha");
    assert_eq!(reports[1].owner.as_str(), "zeta");

    let report = &reports[0];
    assert_eq!(report.incident_count, (MAX_BOOT_REPORT_DETAILS + 1) as u64);
    assert_eq!(
        report.migration_plan_count,
        (MAX_BOOT_REPORT_DETAILS + 1) as u64
    );
    assert_eq!(report.incidents.len(), MAX_BOOT_REPORT_DETAILS);
    assert_eq!(report.migration_plans.len(), MAX_BOOT_REPORT_DETAILS);
    assert!(report.details_truncated);
    assert!(
        report
            .incidents
            .iter()
            .all(|incident| incident.owner == report.owner)
    );
}

#[test]
fn module_command_carries_module_identity() {
    let (mut engine, nick) = setup();
    run(&mut engine, nick, "+whoami");
    assert_eq!(
        toy(&engine).whoami,
        Some(Some(SmolStr::new("toy"))),
        "module commands run with their module identity"
    );
}

// ----------------------------------------------------------------------
// Alias stage
// ----------------------------------------------------------------------

#[test]
fn alias_stage_handles_and_rewrites() {
    let (mut engine, nick) = setup();
    // Handled: consumed by the stage, no Huh.
    let out = run(&mut engine, nick, "toyecho hi there");
    assert!(sys_texts(&out).iter().any(|t| t == "echo: hi there"));
    // Rewrite: becomes a say, observed by the Said{Say} subscription.
    let out = run(&mut engine, nick, "toyrw rewritten words");
    assert!(out.events().iter().any(|(_, e)| matches!(
        e,
        OutputEvent::Narrative { kind: NarrativeKind::Say, text, .. }
            if text.to_plain_text() == "rewritten words"
    )));
    assert_eq!(toy(&engine).said, vec!["rewritten words".to_string()]);
}

/// Prefix-expanded lines never reach the alias stage: an alias that claims
/// the verb `say` (what `addcom say=Public` sets up) captures a *typed*
/// `say trapme`, but `"trapme` stays private room speech — the prefix
/// character settled the line's meaning before aliases run.
#[test]
fn prefix_expanded_lines_skip_the_alias_stage() {
    let (mut engine, nick) = setup();
    // Typed verb: the alias stage wins (aliases-before-command-table).
    let out = run(&mut engine, nick, "say trapme");
    assert!(sys_texts(&out).iter().any(|t| t == "alias captured say"));
    assert!(toy(&engine).said.is_empty());
    // Prefix form: an ordinary room say, never offered to the stage.
    let out = run(&mut engine, nick, "\"trapme");
    assert!(
        !sys_texts(&out).iter().any(|t| t == "alias captured say"),
        "prefix-expanded speech must not be captured by an alias stage"
    );
    assert!(out.events().iter().any(|(_, e)| matches!(
        e,
        OutputEvent::Narrative { kind: NarrativeKind::Say, text, .. }
            if text.to_plain_text() == "trapme"
    )));
    assert_eq!(toy(&engine).said, vec!["trapme".to_string()]);
}

// ----------------------------------------------------------------------
// Marker flags + attr namespace enforcement
// ----------------------------------------------------------------------

#[test]
fn marker_flags_set_and_clear_via_at_set() {
    let (mut engine, _) = setup();
    let ball = engine
        .world
        .create(ObjKind::Thing, "ball", GOD, Some(LIMBO))
        .unwrap();
    let out = run(&mut engine, GOD, "@set ball=toy:shiny");
    assert!(sys_texts(&out).iter().any(|t| t == "Set."));
    assert!(engine.world.has_marker(ball, "toy:shiny"));
    // Clearing with !.
    let out = run(&mut engine, GOD, "@set ball=!TOY:SHINY");
    assert!(sys_texts(&out).iter().any(|t| t == "Cleared."));
    assert!(!engine.world.has_marker(ball, "toy:shiny"));
    // Unregistered marker names still fail like unknown flags.
    let out = run(&mut engine, GOD, "@set ball=toy:bogus");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("I don't understand the flag"))
    );
}

#[test]
fn attr_namespace_enforced_in_at_set() {
    let (mut engine, nick) = setup();
    // A plain player may not write into a module-owned namespace...
    let out = run(&mut engine, nick, "@set me/toy:mood=grim");
    assert!(
        sys_texts(&out).iter().any(|t| t.contains("module-owned")),
        "non-owner write must be denied: {:?}",
        sys_texts(&out)
    );
    assert!(engine.world.attr(nick, "toy:mood").is_none());
    // ...nor clear one...
    engine
        .world
        .set_attr(
            nick,
            "toy:mood",
            mush_core::Attr::text("set by module", GOD),
        )
        .unwrap();
    let out = run(&mut engine, nick, "@set me/toy:mood=");
    assert!(sys_texts(&out).iter().any(|t| t.contains("module-owned")));
    assert!(engine.world.attr(nick, "toy:mood").is_some());
    // ...but unnamespaced attrs are untouched by the rule...
    let out = run(&mut engine, nick, "@set me/mood=cheery");
    assert!(sys_texts(&out).iter().any(|t| t == "mood - Set."));
    // ...Wizards pass...
    let out = run(&mut engine, GOD, "@set Nick/toy:mood=blessed");
    assert!(sys_texts(&out).iter().any(|t| t == "toy:mood - Set."));
    // ...and the owning module identity passes the check itself.
    let reg = &engine.registry;
    assert!(reg.may_write_attr(&engine.world, nick, Some("toy"), "toy:mood"));
    assert!(!reg.may_write_attr(&engine.world, nick, Some("other"), "toy:mood"));
    assert!(!reg.may_write_attr(&engine.world, nick, None, "toy:mood"));
    assert!(reg.may_write_attr(&engine.world, nick, None, "plain"));
    assert!(reg.may_write_attr(&engine.world, nick, None, "detail:window"));
}

// ----------------------------------------------------------------------
// Ticks
// ----------------------------------------------------------------------

#[test]
fn coarse_and_fast_ticks_fire_and_rearm() {
    let (mut engine, _) = setup();
    // Armed at construction relative to now=1000s: fast at +250ms.
    assert_eq!(engine.sched.next_due(), Some(1_000_250));
    let _ = fire(&mut engine, 1_000_250);
    assert_eq!((toy(&engine).fast_ticks, toy(&engine).coarse_ticks), (1, 0));
    // Re-armed: next fast due one period after the fire.
    assert_eq!(engine.sched.next_due(), Some(1_000_500));
    // At +5s both fire (one fast catch-up per fire_due call, no bursts).
    let _ = fire(&mut engine, 1_005_000);
    assert_eq!((toy(&engine).fast_ticks, toy(&engine).coarse_ticks), (2, 1));
    // Coarse re-armed 5s after ITS fire.
    let _ = fire(&mut engine, 1_010_000);
    assert_eq!(toy(&engine).coarse_ticks, 2);
}

// ----------------------------------------------------------------------
// MoveAttempt veto + LookRendered
// ----------------------------------------------------------------------

/// Rooms + exits: Limbo --barred--> Vault (vetoed), Limbo --east--> Hall.
fn dig_test_rooms(engine: &mut Engine) -> (ObjId, ObjId) {
    let w = &mut engine.world;
    let vault = w.create(ObjKind::Room, "Vault", GOD, None).unwrap();
    let hall = w.create(ObjKind::Room, "Hall", GOD, None).unwrap();
    let barred = w.create(ObjKind::Exit, "barred", GOD, Some(LIMBO)).unwrap();
    w.link_exit(barred, Some(vault)).unwrap();
    let east = w.create(ObjKind::Exit, "east;e", GOD, Some(LIMBO)).unwrap();
    w.link_exit(east, Some(hall)).unwrap();
    (vault, hall)
}

#[test]
fn move_attempt_veto_blocks_exit_with_reason() {
    let (mut engine, nick) = setup();
    let (_vault, hall) = dig_test_rooms(&mut engine);
    // Vetoed: reason shown, nobody moves, no Moved event.
    let out = run(&mut engine, nick, "barred");
    assert!(sys_texts(&out).iter().any(|t| t == "The wards flare."));
    assert_eq!(engine.world.get(nick).unwrap().location, Some(LIMBO));
    assert!(toy(&engine).moved.is_empty());
    // No departure narrative leaked to the room.
    assert!(!out.events().iter().any(
        |(_, e)| matches!(e, OutputEvent::Narrative { text, .. } if text.to_plain_text().contains("goes"))
    ));
    // A non-vetoed exit works and notifies observers.
    let out = run(&mut engine, nick, "east");
    assert_eq!(engine.world.get(nick).unwrap().location, Some(hall));
    assert_eq!(toy(&engine).moved, vec![(nick, hall)]);
    // The arrival room render carries the LookRendered hook line.
    let texts = sys_texts(&out);
    assert!(
        texts.iter().any(|t| t == "A toy glows here."),
        "LookRendered line missing from arrival: {texts:?}"
    );
    let room_idx = out
        .events()
        .iter()
        .position(|(_, e)| matches!(e, OutputEvent::Room(_)))
        .expect("room view");
    let line_idx = out
        .events()
        .iter()
        .position(
            |(_, e)| matches!(e, OutputEvent::Sys(t) if t.to_plain_text() == "A toy glows here."),
        )
        .unwrap();
    assert!(line_idx > room_idx, "hook lines append after the room view");
}

#[test]
fn look_appends_hook_lines() {
    let (mut engine, nick) = setup();
    let out = run(&mut engine, nick, "look");
    let texts = sys_texts(&out);
    assert!(texts.iter().any(|t| t == "A toy glows here."));
}

#[test]
fn teleport_consults_move_attempt() {
    let (mut engine, _) = setup();
    let (vault, hall) = dig_test_rooms(&mut engine);
    let out = run(&mut engine, GOD, &format!("@teleport {vault}"));
    assert!(
        sys_texts(&out).iter().any(|t| t == "The wards flare."),
        "veto reason must be shown: {:?}",
        sys_texts(&out)
    );
    assert_eq!(engine.world.get(GOD).unwrap().location, Some(LIMBO));
    // Allowed teleport still notifies Moved observers.
    run(&mut engine, GOD, &format!("@teleport {hall}"));
    assert_eq!(engine.world.get(GOD).unwrap().location, Some(hall));
    assert_eq!(toy(&engine).moved, vec![(GOD, hall)]);
}

#[test]
fn home_notifies_moved_observers() {
    let (mut engine, nick) = setup();
    let (_, hall) = dig_test_rooms(&mut engine);
    engine.world.move_to(nick, hall).unwrap();
    run(&mut engine, nick, "home");
    assert_eq!(engine.world.get(nick).unwrap().location, Some(LIMBO));
    assert_eq!(toy(&engine).moved, vec![(nick, LIMBO)]);
}

/// `home` consults the MoveAttempt veto like every other move: a vetoed
/// destination cannot be walked into by setting (or having) it as home —
/// a jail stays a jail, a locked battle hex stays locked.
#[test]
fn home_consults_move_attempt() {
    let (mut engine, nick) = setup();
    let (vault, _) = dig_test_rooms(&mut engine);
    engine.world.set_home(nick, Some(vault)).unwrap();
    let out = run(&mut engine, nick, "home");
    assert!(
        sys_texts(&out).iter().any(|t| t == "The wards flare."),
        "the veto reason must be shown: {:?}",
        sys_texts(&out)
    );
    assert_eq!(engine.world.get(nick).unwrap().location, Some(LIMBO));
    assert!(toy(&engine).moved.is_empty(), "no Moved event for a veto");
}

/// `@dig/teleport` is a move like any other: vetoable and observed.
#[test]
fn dig_teleport_consults_move_attempt_and_notifies() {
    let (mut engine, _) = setup();
    // Allowed: the digger lands in the new room and observers hear it.
    run(&mut engine, GOD, "@dig/teleport Workshop");
    let ws = engine
        .world
        .iter()
        .find(|o| o.name == "Workshop")
        .expect("dug room")
        .id;
    assert_eq!(engine.world.get(GOD).unwrap().location, Some(ws));
    assert!(toy(&engine).moved.contains(&(GOD, ws)));
    // Vetoed: digging a room named "Vault" (the toy hook's veto target)
    // creates it but refuses the teleport.
    let before = toy(&engine).moved.len();
    let out = run(&mut engine, GOD, "@dig/teleport Vault");
    assert!(sys_texts(&out).iter().any(|t| t == "The wards flare."));
    assert_eq!(engine.world.get(GOD).unwrap().location, Some(ws));
    assert_eq!(toy(&engine).moved.len(), before);
}

// ----------------------------------------------------------------------
// Observers: speech, create/destroy, connect/disconnect
// ----------------------------------------------------------------------

#[test]
fn observers_see_say_but_only_subscribed_kinds() {
    let (mut engine, nick) = setup();
    run(&mut engine, nick, "say hello world");
    run(&mut engine, nick, ":grins.");
    run(&mut engine, nick, "\"quoted too");
    assert_eq!(
        toy(&engine).said,
        vec!["hello world".to_string(), "quoted too".to_string()],
        "pose is not subscribed; both say forms are"
    );
}

#[test]
fn observers_see_created_and_destroyed() {
    let (mut engine, _) = setup();
    run(&mut engine, GOD, "@create widget");
    let widget = engine.world.resolve_player("widget"); // things aren't indexed
    assert!(widget.is_none());
    let created = toy(&engine).created.clone();
    assert_eq!(created.len(), 1);
    let widget = created[0];
    assert_eq!(engine.world.get(widget).unwrap().name, "widget");
    run(&mut engine, GOD, "@destroy widget");
    assert_eq!(toy(&engine).destroyed, vec![widget]);
    // @dig and @open notify too.
    run(&mut engine, GOD, "@dig Den=down;d,up;u");
    assert_eq!(toy(&engine).created.len(), 4, "room + two exits + widget");
}

#[test]
fn connection_hooks_and_observers_fire_on_transitions() {
    let (mut engine, nick) = setup();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::ConnUp {
            conn: 2,
            actor: nick,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
    assert_eq!(toy(&engine).hook_connected, vec![nick]);
    assert_eq!(toy(&engine).ev_connected, vec![nick]);
    // A second connection is no transition.
    engine.step(
        Inbound::ConnUp {
            conn: 3,
            actor: nick,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
    assert_eq!(toy(&engine).hook_connected, vec![nick]);
    // Losing one of two: nothing. Losing the last (unclean): link-dead,
    // still connected world-side, so still no Disconnected.
    for conn in [2, 3] {
        engine.step(
            Inbound::ConnDown {
                conn,
                actor: nick,
                reason: mush_core::DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
    }
    assert!(toy(&engine).hook_disconnected.is_empty());
    // Linkdead revival is not a new connection transition.
    engine.step(
        Inbound::ConnUp {
            conn: 4,
            actor: nick,
            caps_structured: false,
            from_linkdead: true,
        },
        &mut out,
        &mut fx,
    );
    assert_eq!(toy(&engine).hook_connected, vec![nick]);
    // Clean quit: full disconnect fires hooks + observers.
    engine.step(
        Inbound::ConnDown {
            conn: 4,
            actor: nick,
            reason: mush_core::DisconnectKind::Quit,
        },
        &mut out,
        &mut fx,
    );
    assert_eq!(toy(&engine).hook_disconnected, vec![nick]);
    assert_eq!(toy(&engine).ev_disconnected, vec![nick]);
}

// ----------------------------------------------------------------------
// State slots, panels, persistence
// ----------------------------------------------------------------------

#[test]
fn state_slot_round_trips() {
    let (mut engine, nick) = setup();
    assert_eq!(toy(&engine).bumps, 0, "declared init installed");
    let out = run(&mut engine, nick, "+bump");
    assert!(sys_texts(&out).iter().any(|t| t == "bumped to 1"));
    run(&mut engine, nick, "+bump");
    assert_eq!(toy(&engine).bumps, 2);
    // Cloning the world clones the state slots (real clone, not a reset).
    let cloned = engine.world.clone();
    assert_eq!(cloned.module_state::<ToyState>().unwrap().bumps, 2);
}

#[test]
fn panel_keyframe_emits_panel_event() {
    let (mut engine, nick) = setup();
    let out = run(&mut engine, nick, "+panel");
    assert!(out.events().iter().any(|(aud, e)| {
        matches!(aud, mush_core::Audience::Actor(a) if *a == nick)
            && matches!(
                e,
                OutputEvent::Panel { feed, kind: PanelFrameKind::Keyframe, data }
                    if feed == "toy.panel" && data == &json!({"hello": 1})
            )
    }));
}

#[test]
fn persist_ops_land_in_engine_queue() {
    let (mut engine, nick) = setup();
    run(&mut engine, nick, "+persist prefs");
    let ops = engine.take_module_ops();
    assert_eq!(
        ops,
        vec![
            ModuleOp {
                ns: "toy".into(),
                op: ModuleWrite::JsonSet {
                    key: "prefs".into(),
                    value: json!({"ok": true}),
                },
            },
            ModuleOp {
                ns: "toy".into(),
                op: ModuleWrite::StreamAppend {
                    key: "history".into(),
                    value: json!("logged"),
                    maxlen: 100,
                },
            },
        ]
    );
    assert!(engine.take_module_ops().is_empty(), "drained");
}

/// The atomic-approval contract: a handler that queues persistence ops and
/// THEN returns `Err` must not commit them — dispatch truncates the
/// deferred queue back on error, so `take_module_ops` yields nothing.
/// (This is what makes "a handler error leaves the job untouched" true
/// for the persist queue, not just the job document.)
#[test]
fn failed_handler_discards_its_deferred_persist_ops() {
    let (mut engine, nick) = setup();
    let out = run(&mut engine, nick, "+failpersist");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("validation failed")),
        "the error is still reported"
    );
    assert!(
        engine.take_module_ops().is_empty(),
        "ops deferred by a failed handler must never reach the WriteBatch"
    );
    // A successful command's ops still flow (the truncation is scoped).
    run(&mut engine, nick, "+persist prefs");
    assert_eq!(engine.take_module_ops().len(), 2);
}

#[test]
fn deferred_persist_rides_the_outbox_side_channel() {
    // The wiring contract itself: persist() queues on the deferred
    // side-channel, like Deferred::Effect.
    let mut world = World::new_minimal();
    let mut reg = Registry::new();
    register_core_commands(&mut reg).unwrap();
    let mut out = Outbox::new();
    let mut ctx = Ctx {
        w: &mut world,
        actor: GOD,
        switches: mush_core::SwitchSet::default(),
        args: mush_core::ParsedArgs::default(),
        out: &mut out,
        reg: &reg,
        module: Some("toy".into()),
    };
    ctx.persist(ModuleOp {
        ns: "toy".into(),
        op: ModuleWrite::Del { key: "gone".into() },
    });
    assert_eq!(out.deferred().len(), 1);
    assert!(matches!(
        &out.deferred()[0],
        mush_core::Deferred::Persist(op) if op.ns == "toy"
    ));
}

// ----------------------------------------------------------------------
// Scenes
// ----------------------------------------------------------------------

#[test]
fn scene_lifecycle_hooks_state_map_and_ticks() {
    let (mut engine, nick) = setup();
    run(&mut engine, nick, "+scenestart");
    let id = toy(&engine).scene.expect("scene started");
    assert_eq!(toy(&engine).scenes_started, vec![id.0]);
    // The scene table.
    let scene = engine.world.scene(id).expect("active scene");
    assert_eq!(scene.kind, "toy.battle");
    assert!(scene.members.contains(&nick));
    assert_eq!(scene.ruleset_state_key, format!("scene:{}:state", id.0));
    // RulesetStateStore + MapBlob in RAM.
    assert_eq!(engine.world.scene_state(id, nick), Some(&json!({"hp": 10})));
    assert_eq!(engine.world.scene_map(id), Some(&json!({"hex": [0, 0]})));
    // Persisted as module ops: a state snapshot + the map blob.
    let ops = engine.take_module_ops();
    assert_eq!(
        ops,
        vec![
            ModuleOp {
                ns: "toy".into(),
                op: ModuleWrite::JsonSet {
                    key: SmolStr::new(format!("scene:{}:state", id.0)),
                    value: json!({ nick.to_string(): {"hp": 10} }),
                },
            },
            ModuleOp {
                ns: "toy".into(),
                op: ModuleWrite::JsonSet {
                    key: SmolStr::new(format!("scene:{}:map", id.0)),
                    value: json!({"hex": [0, 0]}),
                },
            },
        ]
    );
    // The scene ticker fires every 100ms and re-arms.
    let _ = fire(&mut engine, 1_000_100);
    assert_eq!(toy(&engine).scene_ticks, 1);
    let _ = fire(&mut engine, 1_000_200);
    assert_eq!(toy(&engine).scene_ticks, 2);
    // Ending the scene fires the hook while it is still queryable, then
    // removes it and queues Del ops for its persisted keys.
    run(&mut engine, nick, "+sceneend");
    assert_eq!(toy(&engine).scenes_ended, vec![id.0]);
    assert!(engine.world.scene(id).is_none());
    assert_eq!(engine.world.scene_state(id, nick), None);
    let ops = engine.take_module_ops();
    assert_eq!(
        ops,
        vec![
            ModuleOp {
                ns: "toy".into(),
                op: ModuleWrite::Del {
                    key: SmolStr::new(format!("scene:{}:state", id.0)),
                },
            },
            ModuleOp {
                ns: "toy".into(),
                op: ModuleWrite::Del {
                    key: SmolStr::new(format!("scene:{}:map", id.0)),
                },
            },
        ]
    );
    // The ticker dies with the scene.
    let _ = fire(&mut engine, 1_000_300);
    let _ = fire(&mut engine, 1_000_400);
    assert_eq!(toy(&engine).scene_ticks, 2);
}

/// Scene persistence is owned by the CREATING module, not the caller: a
/// module-less (core/staff) context ending the scene still deletes the
/// documents under the owner's namespace — nothing orphans.
#[test]
fn scene_persistence_uses_owning_namespace_regardless_of_caller() {
    let (mut engine, nick) = setup();
    run(&mut engine, nick, "+scenestart");
    let id = toy(&engine).scene.expect("scene started");
    assert_eq!(engine.world.scene(id).unwrap().ns, "toy");
    let _ = engine.take_module_ops();
    // End it from a context with NO module identity (a staff tool, say).
    let mut out = Outbox::new();
    let mut ctx = Ctx {
        w: &mut engine.world,
        actor: GOD,
        switches: mush_core::SwitchSet::default(),
        args: mush_core::ParsedArgs::default(),
        out: &mut out,
        reg: &engine.registry,
        module: None,
    };
    assert!(ctx.end_scene(id));
    // Route the deferred ops through the engine's queue.
    let deferred = out.take_deferred();
    let dels: Vec<&ModuleOp> = deferred
        .iter()
        .filter_map(|d| match d {
            mush_core::Deferred::Persist(op) => Some(op),
            _ => None,
        })
        .collect();
    assert_eq!(dels.len(), 2);
    assert!(
        dels.iter().all(|op| op.ns == "toy"),
        "scene deletes must land in the OWNER's namespace: {dels:?}"
    );
}

#[test]
fn scene_ticks_stop_on_demand() {
    let (mut engine, nick) = setup();
    run(&mut engine, nick, "+scenestart");
    let _ = engine.take_module_ops();
    let _ = fire(&mut engine, 1_000_100);
    assert_eq!(toy(&engine).scene_ticks, 1);
    // Dynamic removal at runtime.
    run(&mut engine, nick, "+scenetickstop");
    let _ = fire(&mut engine, 1_000_200);
    let _ = fire(&mut engine, 1_000_300);
    assert_eq!(toy(&engine).scene_ticks, 1, "stopped ticks never fire");
    // The scene itself is still alive.
    assert!(engine.world.scene(toy(&engine).scene.unwrap()).is_some());
}

#[test]
fn scene_tick_start_validates_registration() {
    let (mut engine, nick) = setup();
    run(&mut engine, nick, "+scenestart");
    let id = toy(&engine).scene.unwrap();
    let mut out = Outbox::new();
    let mut ctx = Ctx {
        w: &mut engine.world,
        actor: nick,
        switches: mush_core::SwitchSet::default(),
        args: mush_core::ParsedArgs::default(),
        out: &mut out,
        reg: &engine.registry,
        module: Some("toy".into()),
    };
    // Unregistered tag refused; "fast" is registered but not Scene-cadence.
    assert!(
        ctx.scene_tick_start(id, "bogus", Duration::from_millis(50))
            .is_err()
    );
    assert!(
        ctx.scene_tick_start(id, "fast", Duration::from_millis(50))
            .is_err()
    );
    // No module identity: refused.
    ctx.module = None;
    assert!(
        ctx.scene_tick_start(id, "battle", Duration::from_millis(50))
            .is_err()
    );
}

// ----------------------------------------------------------------------
// Input contexts
// ----------------------------------------------------------------------

#[test]
fn input_context_captures_lines_before_everything() {
    let (mut engine, nick) = setup();
    let out = run(&mut engine, nick, "+compose");
    // Pushing emitted the context's prompt.
    let ps = prompts(&out);
    assert_eq!(ps.len(), 1);
    assert_eq!(ps[0].text.as_ref().unwrap().to_plain_text(), "compose>");
    // Lines feed the context — even ones that look like commands, prefix
    // chars, or exits.
    run(&mut engine, nick, "look");
    run(&mut engine, nick, "\"hello");
    run(&mut engine, nick, "home");
    assert_eq!(
        toy(&engine).compose_lines,
        vec![
            "look".to_string(),
            "\"hello".to_string(),
            "home".to_string()
        ]
    );
    assert!(toy(&engine).said.is_empty(), "nothing reached dispatch");
    // "." pops; the normal prompt is restored.
    let out = run(&mut engine, nick, ".");
    let ps = prompts(&out);
    assert_eq!(ps.len(), 1);
    assert_eq!(ps[0].mode, InputMode::Normal);
    assert!(ps[0].text.is_none());
    assert_eq!(engine.world.input_depth(nick), 0);
    // Dispatch is back to normal.
    run(&mut engine, nick, "say freed");
    assert_eq!(toy(&engine).said, vec!["freed".to_string()]);
}

#[test]
fn tilde_q_always_escapes() {
    let (mut engine, nick) = setup();
    run(&mut engine, nick, "+compose");
    run(&mut engine, nick, "captured");
    let out = run(&mut engine, nick, "~q");
    assert!(sys_texts(&out).iter().any(|t| t == "Aborted."));
    assert_eq!(engine.world.input_depth(nick), 0);
    // The context never saw the "~q" line.
    assert_eq!(toy(&engine).compose_lines, vec!["captured".to_string()]);
    // Prompt restored to normal.
    assert!(prompts(&out).iter().any(|p| p.mode == InputMode::Normal));
}

/// Reconnecting (or multi-connecting) while an input context is live
/// re-sends the context's prompt to the NEW connection: without it, a
/// player who went linkdead mid-compose reconnects into a normal-mode
/// editor and types commands into their mail body.
#[test]
fn conn_up_resyncs_prompt_into_live_input_context() {
    let (mut engine, nick) = setup();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::ConnUp {
            conn: 2,
            actor: nick,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
    run(&mut engine, nick, "+compose");
    // Linkdead drop, then revival on a new connection.
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::ConnDown {
            conn: 2,
            actor: nick,
            reason: mush_core::DisconnectKind::Lost,
        },
        &mut out,
        &mut fx,
    );
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::ConnUp {
            conn: 3,
            actor: nick,
            caps_structured: false,
            from_linkdead: true,
        },
        &mut out,
        &mut fx,
    );
    let prompt = out
        .events()
        .iter()
        .find_map(|(aud, e)| match (aud, e) {
            (mush_core::Audience::Conn(3), OutputEvent::Prompt(p)) => Some(p.clone()),
            _ => None,
        })
        .expect("the new connection must receive the context prompt");
    assert_eq!(prompt.text.as_ref().unwrap().to_plain_text(), "compose>");
    // A plain connect with NO live context pushes no prompt.
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    run(&mut engine, nick, "."); // pop the context
    engine.step(
        Inbound::ConnUp {
            conn: 4,
            actor: nick,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
    assert!(
        !out.events()
            .iter()
            .any(|(aud, e)| matches!(aud, mush_core::Audience::Conn(4))
                && matches!(e, OutputEvent::Prompt(_))),
        "no context, no prompt push"
    );
}

/// Input contexts run with the identity of the module that pushed them:
/// the toy module's compose context sees `ctx.module == Some("toy")` on
/// every fed line, not `None`.
#[test]
fn input_context_carries_pushing_module_identity() {
    let (mut engine, nick) = setup();
    run(&mut engine, nick, "+compose");
    // ToyCompose's feed records into the state slot via `state(ctx)`; use
    // a fed line that ALSO checks the module identity via whoami below.
    run(&mut engine, nick, "run +whoami");
    assert_eq!(
        toy(&engine).whoami,
        Some(Some(SmolStr::new("toy"))),
        "PopAndRun re-dispatch keeps working"
    );
    // Direct check on the feed path itself.
    run(&mut engine, nick, "+compose");
    let stacked = engine.world.pop_input_context(nick).expect("context");
    assert_eq!(stacked.module.as_deref(), Some("toy"));
}

#[test]
fn pop_and_run_dispatches_through_the_pipeline() {
    let (mut engine, nick) = setup();
    run(&mut engine, nick, "+compose");
    let out = run(&mut engine, nick, "run say sent from compose");
    assert_eq!(engine.world.input_depth(nick), 0);
    assert!(out.events().iter().any(|(_, e)| matches!(
        e,
        OutputEvent::Narrative { kind: NarrativeKind::Say, text, .. }
            if text.to_plain_text() == "sent from compose"
    )));
    assert_eq!(toy(&engine).said, vec!["sent from compose".to_string()]);
}

// ----------------------------------------------------------------------
// Boot data
// ----------------------------------------------------------------------

#[test]
fn on_world_loaded_receives_boot_data_and_may_persist() {
    let (mut engine, _) = setup();
    let mut boot = std::collections::HashMap::new();
    boot.insert(
        SmolStr::new("toy"),
        ModuleBootData {
            kv: vec![(SmolStr::new("cfg"), json!({"a": 1}))],
            streams: std::collections::HashMap::from([(
                SmolStr::new("history"),
                vec![
                    ("1-0".to_string(), json!("first")),
                    ("2-0".to_string(), json!("second")),
                ],
            )]),
        },
    );
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.modules_loaded(boot, &mut out, &mut fx).unwrap();
    assert_eq!(
        toy(&engine).boot_kv,
        vec![(SmolStr::new("cfg"), json!({"a": 1}))]
    );
    assert_eq!(toy(&engine).boot_stream_len, 2);
    // Ops deferred during boot are routed like a command's.
    let ops = engine.take_module_ops();
    assert_eq!(
        ops,
        vec![ModuleOp {
            ns: "toy".into(),
            op: ModuleWrite::JsonSet {
                key: "booted".into(),
                value: json!(true),
            },
        }]
    );
}

#[test]
fn modules_loaded_defaults_missing_boot_data() {
    let (mut engine, _) = setup();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(std::collections::HashMap::new(), &mut out, &mut fx)
        .unwrap();
    assert!(toy(&engine).boot_kv.is_empty());
    assert_eq!(toy(&engine).boot_stream_len, 0);
}

// ----------------------------------------------------------------------
// Contract shapes
// ----------------------------------------------------------------------

#[test]
fn module_op_serde_round_trips() {
    for op in [
        ModuleOp {
            ns: "comsys".into(),
            op: ModuleWrite::JsonSet {
                key: "chan:pub".into(),
                value: json!({"name": "Public"}),
            },
        },
        ModuleOp {
            ns: "mail".into(),
            op: ModuleWrite::Del {
                key: "msg:9".into(),
            },
        },
        ModuleOp {
            ns: "comsys".into(),
            op: ModuleWrite::StreamAppend {
                key: "chan:pub:history".into(),
                value: json!("hi"),
                maxlen: 200,
            },
        },
    ] {
        let s = serde_json::to_string(&op).unwrap();
        let back: ModuleOp = serde_json::from_str(&s).unwrap();
        assert_eq!(back, op);
    }
}
