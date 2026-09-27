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
    ContextOutcome, EventHook, GameModule, HookPoint, HookReadContext, InputContext, JobQueueSpec,
    Migration, ModError, ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, PanelFeedHandle,
    QueryHandler, RecoveryReporter, Registrar, SceneId, TickCadence, WorldEvent, WorldEventKind,
};
use mush_core::{
    ActionError, ActionInvocation, ActionProvider, AliasOutcome, AliasStage, ArgShape,
    AuthorityGuard, AuthorityGuardKind, AuthorityGuardValidator, BehaviorCommand, BehaviorId,
    BehaviorSpec, BehaviorTarget, BridgeDescriptor, CanonicalId, CmdError, CommandSpec, Ctx,
    DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind,
    DefinitionRef, DefinitionValidationContext, GOD, IncidentId, IncidentState, LIMBO,
    MAX_BOOT_REPORT_DETAILS, MigrationPlanSummary, ModuleBootReport, ObjId, ObjKind, Outbox,
    OwnerId, RecoveryIncidentSummary, RecoveryReason, RecoveryScope, RecoverySeverity, Registry,
    RevisionKey, RevisionKind, RevisionValidator, StateSlotDenied, SubjectKey, World,
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
        ctx: &HookReadContext<'_>,
        _actor: ObjId,
        _exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), RichText> {
        if ctx.world().get(dest).is_some_and(|o| o.name == "Vault") {
            return Err(RichText::plain("The wards flare."));
        }
        Ok(())
    }

    fn look_rendered(
        &self,
        _ctx: &HookReadContext<'_>,
        _looker: ObjId,
        _room: ObjId,
    ) -> Vec<RichText> {
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
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        // Typed state slot.
        reg.state(ToyState::default());

        // Markers, attr namespace, script api, job queue, migrations.
        reg.marker_flag("toy:shiny")?;
        reg.attr_namespace("toy")?;
        // 16b cut D.5: `look <keyword>` also reads this module's own detail
        // shelf, which it may only point at because it just claimed `toy`.
        reg.detail_prefix("toy:detail:")?;
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
        reg.alias_stage(Arc::new(ToyAliases))?;

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

#[test]
fn composed_module_commands_require_an_explicit_compatibility_class() {
    struct Unclassified;

    impl GameModule for Unclassified {
        fn name(&self) -> &'static str {
            "unclassified"
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.command(
                CommandSpec {
                    name: "+unsafe-default".into(),
                    ..CommandSpec::default()
                },
                Arc::new(|_ctx: &mut Ctx<'_>| Ok(())),
            )
        }
    }

    let error = match Engine::compose_modules(Registry::new(), vec![Box::new(Unclassified)]) {
        Ok(_) => panic!("an unclassified composed command must fail"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains(
            "module unclassified: classify commands before registering \"+unsafe-default\""
        )
    );
}

#[test]
fn frozen_registry_exposes_command_compatibility_by_name_and_alias() {
    let (engine, _) = setup();
    assert_eq!(
        engine.registry.compatibility_of("+persist"),
        Some(mush_core::CommandCompatibility::LegacyPostImage)
    );
    assert_eq!(
        engine
            .registry
            .specs_with_compatibility()
            .find(|(spec, _)| spec.name == "+persist")
            .map(|(_, compatibility)| compatibility),
        Some(mush_core::CommandCompatibility::LegacyPostImage)
    );
    assert_eq!(
        engine
            .registry
            .alias_stage_compatibilities()
            .collect::<Vec<_>>(),
        // The kernel's macro stage (16b cut AD.1) is registered by
        // `register_core_commands`, so it is first, as host/core (`None`).
        vec![
            (None, mush_core::CommandCompatibility::LegacyPostImage),
            (
                Some("toy"),
                mush_core::CommandCompatibility::LegacyPostImage
            )
        ]
    );
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
    // Panel feeds, query handlers, job queues, script apis, migrations. Core
    // registers its own `commands.list` feed (interface plan I1) beside the
    // module's, so this asserts membership, not the whole table.
    assert!(
        reg.panel_feeds()
            .contains(&(SmolStr::new("toy.panel"), SmolStr::new("toy"))),
        "toy.panel registered under toy: {:?}",
        reg.panel_feeds()
    );
    assert!(
        reg.panel_feeds()
            .iter()
            .any(|(feed, owner)| feed == "commands.list" && owner == "core"),
        "core's commands.list feed present: {:?}",
        reg.panel_feeds()
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

// ----------------------------------------------------------------------
// WS03-S8 / WS03-S14: state-slot ownership, enforced
// ----------------------------------------------------------------------

/// Both ownership tests that read the log run under a thread-scoped
/// subscriber, and `tracing` registers a callsite lazily on its first hit:
/// while exactly one dispatcher is registered it evaluates that callsite's
/// interest against the *registering thread's* default subscriber, so a
/// sibling hitting the refusal first on a thread with no subscriber would
/// cache it as never-enabled and the capturing test would see nothing.
/// Serialising them removes the interleaving (the shape `mush-config`'s
/// `ipc_endpoint_validation.rs` documents).
static CENSUS_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn census_serialized() -> std::sync::MutexGuard<'static, ()> {
    CENSUS_SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A `MakeWriter` collecting formatted log lines into a shared buffer.
#[derive(Clone, Default)]
struct LogCapture(Arc<std::sync::Mutex<Vec<u8>>>);

impl LogCapture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("log buffer")).into_owned()
    }
}

impl std::io::Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("log buffer").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
    type Writer = LogCapture;
    fn make_writer(&'a self) -> LogCapture {
        self.clone()
    }
}

/// Run `f` with a scoped subscriber; return its result and what it logged.
fn with_captured_logs<T>(f: impl FnOnce() -> T) -> (T, String) {
    let capture = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(capture.clone())
        .finish();
    let out = tracing::subscriber::with_default(subscriber, f);
    (out, capture.text())
}

/// The slot module `a` declares.
#[derive(Clone, Debug, Default)]
struct OwnedSlot(u32);

/// The slot module `b` declares, in this very same crate.
#[derive(Clone, Debug, Default)]
struct PeerSlot(u32);

/// A slot nobody declares — seeded straight onto the world.
#[derive(Clone, Debug, Default)]
struct SeededSlot(u32);

/// Module `a`: declares [`OwnedSlot`] and bumps its own slot at boot.
struct SlotOwner;

impl GameModule for SlotOwner {
    fn name(&self) -> &'static str {
        "a"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(OwnedSlot::default());
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        _data: ModuleBootData,
    ) -> Result<(), ModError> {
        ctx.try_state_mut::<OwnedSlot>()
            .expect("its own slot is never refused")
            .expect("its own slot is installed")
            .0 += 1;
        Ok(())
    }
}

/// What a module saw when it reached for a slot it does not own.
#[derive(Default)]
struct PoachReport {
    /// The refusal `try_state_mut` returned (`None` if it was allowed).
    denied: Option<StateSlotDenied>,
    /// Whether `state_mut` handed out a live `&mut` — it must not.
    state_mut_was_live: bool,
    /// What the poacher could still READ: reading stays shared.
    read: Option<u32>,
    /// The value it wrote into a slot it DOES own, if it owns one.
    own_slot: Option<u32>,
}

/// Module `b`: mutates `a`'s slot at boot. `declare_own` makes it a module
/// with state of its own — the same crate, a different owner.
struct SlotPoacher {
    declare_own: bool,
    report: Arc<std::sync::Mutex<PoachReport>>,
}

impl SlotPoacher {
    fn new(declare_own: bool) -> (Self, Arc<std::sync::Mutex<PoachReport>>) {
        let report = Arc::new(std::sync::Mutex::new(PoachReport::default()));
        (
            Self {
                declare_own,
                report: Arc::clone(&report),
            },
            report,
        )
    }
}

impl GameModule for SlotPoacher {
    fn name(&self) -> &'static str {
        "b"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        if self.declare_own {
            reg.state(PeerSlot::default());
        }
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        _data: ModuleBootData,
    ) -> Result<(), ModError> {
        let read = ctx.state::<OwnedSlot>().map(|s| s.0);
        // Twice, because the refusal is reported once per (module, slot) and
        // refused every time.
        let mut denied = None;
        for _ in 0..2 {
            match ctx.try_state_mut::<OwnedSlot>() {
                Ok(slot) => {
                    if let Some(slot) = slot {
                        slot.0 += 10;
                    }
                }
                Err(err) => denied = Some(err),
            }
        }
        let mut state_mut_was_live = false;
        if let Some(slot) = ctx.state_mut::<OwnedSlot>() {
            slot.0 += 100;
            state_mut_was_live = true;
        }
        let own_slot = if self.declare_own {
            ctx.try_state_mut::<PeerSlot>()
                .expect("its own slot is never refused")
                .map(|slot| {
                    slot.0 += 5;
                    slot.0
                })
        } else {
            None
        };
        *self.report.lock().expect("the report") = PoachReport {
            denied,
            state_mut_was_live,
            read,
            own_slot,
        };
        Ok(())
    }
}

/// **The refusal.** A module mutating another module's slot is refused —
/// `try_state_mut` returns `Err(StateSlotDenied { requester, owner, .. })`,
/// `state_mut` returns `None`, and the mutation does NOT land. Before
/// WS03-S14 it got a live `&mut` and the write stuck.
///
/// The refusal is loud in three places at once, because a denial is invisible
/// at the call site (`if let Some(s) = ctx.state_mut::<T>()` becomes a
/// no-op): the typed error, one log line per (module, slot), and the record
/// on the world.
#[test]
fn a_foreign_module_cannot_mutate_another_modules_slot() {
    let _serial = census_serialized();
    let (poacher, report) = SlotPoacher::new(false);
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(SlotOwner), Box::new(poacher)],
    )
    .expect("two modules, one slot declaration");
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    let (loaded, logs) = with_captured_logs(|| {
        engine.modules_loaded(std::collections::HashMap::new(), &mut out, &mut fx)
    });
    loaded.expect("a refused write must not fail the boot");

    // `a`'s own bump, and nothing of `b`'s three attempts.
    assert_eq!(
        engine.world.module_state::<OwnedSlot>().expect("slot").0,
        1,
        "a foreign write must not land"
    );
    let report = report.lock().expect("the report");
    let denied = report.denied.clone().expect("b must be refused");
    assert_eq!(denied.requester.as_str(), "b");
    assert_eq!(denied.owner.as_str(), "a");
    assert!(denied.slot.ends_with("OwnedSlot"), "{:?}", denied.slot);
    assert!(
        denied.to_string().contains("OwnedSlot"),
        "the refusal names the slot: {denied}"
    );
    assert!(
        !report.state_mut_was_live,
        "state_mut must not hand out a foreign slot"
    );
    // A's write is visible to BOTH modules' reads: reading stays shared.
    assert_eq!(report.read, Some(1), "reading a foreign slot is allowed");
    assert_eq!(
        engine.world.module_state::<OwnedSlot>().map(|s| s.0),
        Some(1)
    );

    // Recorded once, naming b — and a writing its OWN slot is not recorded.
    let refused = engine.world.refused_state_writes();
    assert_eq!(refused.len(), 1, "one pair, thrice attempted: {refused:?}");
    assert_eq!(refused[0].0.as_str(), "b");
    assert!(
        refused[0].1.ends_with("OwnedSlot"),
        "the record names the slot: {:?}",
        refused[0].1
    );
    let refusals = logs.matches("foreign state-slot write REFUSED").count();
    assert_eq!(
        refusals, 1,
        "one line per (module, slot), not one per attempt: {logs}"
    );
    assert!(logs.contains("module=b"), "{logs}");
    assert!(logs.contains("owner=a"), "{logs}");
    assert!(logs.contains("OwnedSlot"), "{logs}");
}

/// Ownership is by MODULE, not by crate: `b` lives in this very file
/// alongside `a` and declares a slot of its own, and is still refused `a`'s.
/// The crate-level half of the enforcement — `mush-contract-tests`'
/// `state-slot-owners.toml` scan — cannot see this boundary at all, which is
/// why the runtime check exists beside it.
#[test]
fn two_modules_in_one_crate_are_checked_separately() {
    // Hits the same refusal `error!` callsite the capturing test above reads;
    // see `census_serialized` for why the two may not interleave.
    let _serial = census_serialized();
    let (poacher, report) = SlotPoacher::new(true);
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(SlotOwner), Box::new(poacher)],
    )
    .expect("two modules in one crate, one slot each");
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(std::collections::HashMap::new(), &mut out, &mut fx)
        .expect("boot");

    let report = report.lock().expect("the report");
    assert_eq!(
        report.own_slot,
        Some(5),
        "a module's own slot is always writable"
    );
    let denied = report.denied.clone().expect("b must still be refused a's");
    assert_eq!(
        (denied.requester.as_str(), denied.owner.as_str()),
        ("b", "a")
    );
    assert_eq!(
        engine.world.module_state::<OwnedSlot>().map(|s| s.0),
        Some(1),
        "same crate, different module: still refused"
    );
    assert_eq!(
        engine.world.module_state::<PeerSlot>().map(|s| s.0),
        Some(5)
    );
    assert_eq!(
        engine
            .world
            .module_state_owner(std::any::TypeId::of::<PeerSlot>())
            .map(SmolStr::as_str),
        Some("b"),
        "one crate, two owners"
    );
}

/// A core command — `module: None` — is a wildcard: the engine's own
/// commands are not a module and are refused nothing.
#[test]
fn core_commands_keep_slot_access() {
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(SlotOwner)],
    )
    .expect("one module composes");
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(std::collections::HashMap::new(), &mut out, &mut fx)
        .expect("boot");

    let mut reg = Registry::new();
    register_core_commands(&mut reg).expect("core commands register");
    let mut out = Outbox::new();
    let mut ctx = Ctx {
        w: &mut engine.world,
        actor: GOD,
        switches: mush_core::SwitchSet::default(),
        args: mush_core::ParsedArgs::default(),
        out: &mut out,
        reg: &reg,
        module: None,
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    ctx.try_state_mut::<OwnedSlot>()
        .expect("a core command is a wildcard")
        .expect("the slot is installed")
        .0 += 9;

    assert_eq!(
        engine.world.module_state::<OwnedSlot>().map(|s| s.0),
        Some(10),
        "a core command's write lands"
    );
    assert!(
        engine.world.refused_state_writes().is_empty(),
        "a wildcard is not a refusal: {:?}",
        engine.world.refused_state_writes()
    );
}

/// A slot seeded only through `World::module_state_insert` has no owner —
/// there is nobody to refuse for — so it stays mutable by every module.
/// That is today's behaviour and the hole ownership cannot see through,
/// which is why boot NAMES those slots (the test below) instead of
/// pretending they are owned.
#[test]
fn an_unowned_slot_stays_open() {
    /// Bumps a slot nobody declared, and records what it got.
    struct SeedWriter(Arc<std::sync::Mutex<Option<Result<u32, StateSlotDenied>>>>);
    impl GameModule for SeedWriter {
        fn name(&self) -> &'static str {
            "b"
        }
        fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), ModError> {
            Ok(())
        }
        fn on_world_loaded(
            &mut self,
            ctx: &mut ModuleCtx<'_>,
            _data: ModuleBootData,
        ) -> Result<(), ModError> {
            let got = ctx.try_state_mut::<SeededSlot>().map(|slot| {
                slot.map_or(0, |slot| {
                    slot.0 += 4;
                    slot.0
                })
            });
            *self.0.lock().expect("the report") = Some(got);
            Ok(())
        }
    }

    let seen = Arc::new(std::sync::Mutex::new(None));
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(SlotOwner), Box::new(SeedWriter(Arc::clone(&seen)))],
    )
    .expect("two modules compose");
    // Seeded before boot, exactly as `on_world_loaded` is sanctioned to do.
    engine.world.module_state_insert(SeededSlot(3));
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(std::collections::HashMap::new(), &mut out, &mut fx)
        .expect("boot");

    assert_eq!(
        seen.lock().expect("the report").clone(),
        Some(Ok(7)),
        "an unowned slot is open to a module that did not declare it"
    );
    assert_eq!(
        engine.world.module_state::<SeededSlot>().map(|s| s.0),
        Some(7),
        "and the write lands"
    );
    assert!(
        engine.world.refused_state_writes().is_empty(),
        "no owner, no refusal"
    );
}

/// A slot seeded only through `World::module_state_insert` (which
/// `on_world_loaded` is explicitly sanctioned to use) has no owner. Boot
/// NAMES it rather than refusing it, because an unowned slot is the hole
/// ownership cannot see through.
#[test]
fn a_slot_with_no_stateinit_owner_is_reported_as_unowned() {
    let _serial = census_serialized();
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(SlotOwner)],
    )
    .expect("one module composes");
    engine.world.module_state_insert(SeededSlot(3));

    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    let (loaded, logs) = with_captured_logs(|| {
        engine.modules_loaded(std::collections::HashMap::new(), &mut out, &mut fx)
    });
    loaded.expect("an unowned slot must never refuse the boot");

    assert!(
        engine
            .world
            .module_state_owner(std::any::TypeId::of::<SeededSlot>())
            .is_none(),
        "a slot nobody declared resolves to no owner"
    );
    let unowned = engine.world.unowned_state_slots();
    assert!(
        unowned.iter().any(|n| n.ends_with("SeededSlot")),
        "{unowned:?}"
    );
    assert!(
        !unowned.iter().any(|n| n.ends_with("OwnedSlot")),
        "a declared slot is owned: {unowned:?}"
    );
    assert!(logs.contains("no declaring module"), "{logs}");
    assert!(logs.contains("SeededSlot"), "{logs}");
    // Still readable, and the slot is unchanged.
    assert_eq!(
        engine.world.module_state::<SeededSlot>().expect("slot").0,
        3
    );
    // Every declared slot resolves to exactly one owner.
    for slot in engine.world.module_state_slots() {
        if slot.type_name.ends_with("OwnedSlot") {
            assert_eq!(slot.owner.as_deref(), Some("a"), "{slot:?}");
        }
    }
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
        _context: &mush_core::RevisionValidatorContext<'_>,
        _key: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        Ok(Some(1))
    }
}

struct ModuleAuthorityValidator;

impl AuthorityGuardValidator for ModuleAuthorityValidator {
    fn is_current(
        &self,
        _context: &mush_core::AuthorityGuardContext<'_>,
        _guard: &AuthorityGuard,
    ) -> Result<bool, ActionError> {
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

/// **16b cut D.5 (16c `B-12`): `look <keyword>` reads a module's own detail
/// shelf**, after the bare `detail:` and never before it.
#[test]
fn look_reads_a_registered_detail_prefix_after_the_bare_detail() {
    let (mut engine, nick) = setup();
    assert_eq!(
        engine
            .registry
            .detail_prefixes()
            .collect::<Vec<_>>()
            .as_slice(),
        ["toy:detail:"],
        "the toy module registered exactly one prefix"
    );
    engine
        .world
        .set_attr(
            LIMBO,
            "toy:detail:sign",
            mush_core::Attr::text("A hand-lettered sign.", GOD),
        )
        .unwrap();
    engine
        .world
        .set_attr(
            LIMBO,
            "toy:detail:coffee_machine",
            mush_core::Attr::text("It takes a while.", GOD),
        )
        .unwrap();
    let out = run(&mut engine, nick, "look sign");
    assert!(
        sys_texts(&out).iter().any(|t| t == "A hand-lettered sign."),
        "{:?}",
        sys_texts(&out)
    );
    // The keyword is folded the way the writer folded it: a space becomes `_`,
    // and the case does not matter.
    for typed in ["look coffee machine", "look COFFEE MACHINE"] {
        let out = run(&mut engine, nick, typed);
        assert!(
            sys_texts(&out).iter().any(|t| t == "It takes a while."),
            "{typed}: {:?}",
            sys_texts(&out)
        );
    }
    // A bare `detail:` of the same keyword still wins.
    engine
        .world
        .set_attr(LIMBO, "detail:sign", mush_core::Attr::text("Plain.", GOD))
        .unwrap();
    let out = run(&mut engine, nick, "look sign");
    assert!(sys_texts(&out).iter().any(|t| t == "Plain."));
    // And a keyword nothing answers to is the ordinary refusal.
    let out = run(&mut engine, nick, "look nosuchthing");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("I don't see that here.")),
        "{:?}",
        sys_texts(&out)
    );
}

/// The seam only admits a prefix inside a namespace the module itself claimed,
/// so `look` can never be pointed at somebody else's attributes.
#[test]
fn detail_prefix_is_refused_outside_the_modules_own_namespaces() {
    struct Greedy(&'static str, bool);

    impl GameModule for Greedy {
        fn name(&self) -> &'static str {
            "greedy"
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
            if self.1 {
                reg.attr_namespace("greedy")?;
            }
            reg.detail_prefix(self.0)
        }
    }

    for (prefix, claims_own, expected) in [
        // Another module's namespace.
        ("toy:detail:", true, "which this module has not registered"),
        // A namespace nobody owns.
        ("wild:detail:", true, "which this module has not registered"),
        // The bare `detail:` is the kernel's own shelf, and `detail` is a
        // namespace nobody claimed.
        ("detail:", true, "which this module has not registered"),
        // No namespace at all.
        (
            "nocolon",
            true,
            "must begin with an attribute namespace this module registered, and a ':'",
        ),
        // Its own namespace, but not claimed first.
        (
            "greedy:detail:",
            false,
            "which this module has not registered",
        ),
    ] {
        let error = match Engine::compose_modules(
            Registry::new(),
            vec![Box::new(ToyModule), Box::new(Greedy(prefix, claims_own))],
        ) {
            Ok(_) => panic!("{prefix} must be refused"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains(expected), "{prefix}: {error}");
    }

    // The same module, claiming first, is accepted — and twice is once.
    struct Polite;

    impl GameModule for Polite {
        fn name(&self) -> &'static str {
            "polite"
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
            reg.attr_namespace("polite")?;
            reg.detail_prefix("polite:detail:")?;
            reg.detail_prefix("polite:detail:")
        }
    }

    let composition =
        Engine::compose_modules(Registry::new(), vec![Box::new(ToyModule), Box::new(Polite)])
            .expect("a module may point look at its own namespace");
    assert_eq!(
        composition
            .registry()
            .detail_prefixes()
            .collect::<Vec<_>>()
            .as_slice(),
        ["toy:detail:", "polite:detail:"],
        "registration order, de-duplicated"
    );
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
        typed_state: mush_core::ModuleActionState::unavailable(),
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
// WS03-A1: cross-namespace module writes
// ----------------------------------------------------------------------

/// A module writing its OWN namespace is the fast path: the op lands and
/// nothing is refused.
#[test]
fn a_module_persisting_into_its_own_namespace_is_untouched() {
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
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    ctx.persist(ModuleOp {
        ns: "toy".into(),
        op: ModuleWrite::Del { key: "gone".into() },
    });
    assert_eq!(out.deferred().len(), 1);
    assert!(
        world.refused_namespace_writes().is_empty(),
        "a module's own namespace is not a cross-namespace write"
    );
}

/// An undeclared foreign namespace is REFUSED: the op is dropped, nothing is
/// queued, and the pair is recorded once however many writes it took.
#[test]
fn an_undeclared_foreign_namespace_persist_is_refused_and_nothing_is_queued() {
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
        module: Some("housing".into()),
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    for key in ["job:1", "job:2"] {
        ctx.persist(ModuleOp {
            ns: "jobs".into(),
            op: ModuleWrite::Del { key: key.into() },
        });
    }
    assert!(
        out.deferred().is_empty(),
        "a refused write queues nothing at all"
    );
    assert_eq!(
        world.refused_namespace_writes(),
        vec![("housing".into(), "jobs".into(), None)],
        "one row per (module, namespace), however many writes it took"
    );
}

/// A core command carries no module identity, and is a wildcard here exactly
/// as it is for state slots.
#[test]
fn a_core_command_is_a_namespace_wildcard() {
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
        module: None,
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    ctx.persist(ModuleOp {
        ns: "jobs".into(),
        op: ModuleWrite::Del {
            key: "job:1".into(),
        },
    });
    assert_eq!(out.deferred().len(), 1);
    assert!(world.refused_namespace_writes().is_empty());
}

/// Composition records `namespace -> owning module`: a module's name IS its
/// namespace, and a namespace only a migration set names belongs to the
/// module that registered it. A namespace nobody claims stays absent.
#[test]
fn composition_records_the_namespace_owner_of_every_module() {
    let (engine, _nick) = setup();
    assert_eq!(
        engine.registry.namespace_owner("toy").map(SmolStr::as_str),
        Some("toy")
    );
    // ToyModule registers migrations for a namespace of its own name only;
    // nothing claims a namespace no module is called after.
    assert_eq!(engine.registry.namespace_owner("jobs"), None);
    let owners = engine.registry.namespace_owners();
    assert!(
        owners
            .iter()
            .any(|(ns, module)| ns == "toy" && module == "toy"),
        "{owners:?} should claim the toy namespace for the toy module"
    );
}

// -- the delegation, end to end ---------------------------------------------

/// A module that owns a namespace nobody else may write undeclared.
struct VaultModule;

impl GameModule for VaultModule {
    fn name(&self) -> &'static str {
        "vault"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        Ok(())
    }
}

/// A producing module that writes the vault's namespace — declared, the way
/// housing declares its reach into `jobs`.
struct ClerkModule;

impl GameModule for ClerkModule {
    fn name(&self) -> &'static str {
        "clerk"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.persist_delegation(
            "clerk",
            "vault",
            "the vault's deposit API takes the caller's Ctx",
        )?;
        reg.command(
            CommandSpec {
                name: "+deposit".into(),
                shape: ArgShape::None,
                help: "test: write a vault document".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                ctx.persist(ModuleOp {
                    ns: "vault".into(),
                    op: ModuleWrite::Del {
                        key: "deposit".into(),
                    },
                });
                Ok(())
            }),
        )
    }
}

/// The same reach, undeclared.
struct ThiefModule;

impl GameModule for ThiefModule {
    fn name(&self) -> &'static str {
        "thief"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            CommandSpec {
                name: "+withdraw".into(),
                shape: ArgShape::None,
                help: "test: write a vault document without a delegation".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                ctx.persist(ModuleOp {
                    ns: "vault".into(),
                    op: ModuleWrite::Del {
                        key: "withdrawal".into(),
                    },
                });
                Ok(())
            }),
        )
    }
}

fn vault_engine() -> (Engine, ObjId) {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    let engine = Engine::with_modules(
        world,
        registry,
        vec![
            Box::new(VaultModule),
            Box::new(ClerkModule),
            Box::new(ThiefModule),
        ],
    )
    .expect("three modules compose");
    (engine, nick)
}

/// **A declared delegation still persists.** This is the case the refusal
/// exists to keep working: the producing module's write into the owning
/// module's namespace lands exactly as it did before.
#[test]
fn a_declared_delegation_still_persists() {
    let (mut engine, nick) = vault_engine();
    run(&mut engine, nick, "+deposit");
    let ops = engine.take_module_ops();
    assert_eq!(ops.len(), 1, "the declared write lands");
    assert_eq!(ops[0].ns, "vault");
    assert!(
        engine.world.refused_namespace_writes().is_empty(),
        "a declared delegation is not a refusal"
    );
}

/// **An undeclared one is dropped**, and says so once.
#[test]
fn an_undeclared_reach_into_another_modules_namespace_is_dropped() {
    let (mut engine, nick) = vault_engine();
    run(&mut engine, nick, "+withdraw");
    assert!(
        engine.take_module_ops().is_empty(),
        "the undeclared write must not reach the write batch"
    );
    assert_eq!(
        engine.world.refused_namespace_writes(),
        vec![("thief".into(), "vault".into(), Some("vault".into()))],
        "the refusal names the writer, the namespace and its owner"
    );
}

/// The delegation is a PAIR, declared by one of the two modules it names: a
/// module cannot declare somebody else's pair, cannot "delegate" to its own
/// namespace, and cannot declare the same pair twice.
#[test]
fn a_delegation_is_declared_by_a_party_to_it_exactly_once() {
    struct Meddler(&'static str, &'static str, &'static str);

    impl GameModule for Meddler {
        fn name(&self) -> &'static str {
            self.0
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
            reg.persist_delegation(self.1, self.2, "test")?;
            reg.persist_delegation(self.1, self.2, "test again")
        }
    }

    // A pair naming neither the declaring module as writer nor as owner.
    let third_party = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Meddler("nosy", "clerk", "vault"))],
    );
    assert!(
        third_party
            .err()
            .expect("a third party may not declare a pair")
            .to_string()
            .contains("neither")
    );

    // A module "delegating" to its own namespace is not a cross-namespace
    // write at all.
    let self_pair = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Meddler("clerk", "clerk", "clerk"))],
    );
    assert!(
        self_pair
            .err()
            .expect("a module's own namespace needs no delegation")
            .to_string()
            .contains("needs no delegation")
    );

    // The same pair twice: the first declaration wins, the second is a
    // conflict.
    let twice = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Meddler("clerk", "clerk", "vault"))],
    );
    assert!(
        twice
            .err()
            .expect("a pair is declared once")
            .to_string()
            .contains("persist delegation")
    );
}

/// A BOOT-phase write gets no exemption: `on_world_loaded` runs with the same
/// module identity a command does, so a module reaching into another module's
/// namespace at boot is the same cross-namespace write and is refused the
/// same way.
#[test]
fn a_boot_phase_write_is_checked_like_any_other() {
    struct BootRaider;

    impl GameModule for BootRaider {
        fn name(&self) -> &'static str {
            "raider"
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
            Ok(())
        }
        fn on_world_loaded(
            &mut self,
            ctx: &mut ModuleCtx<'_>,
            _data: ModuleBootData,
        ) -> Result<(), ModError> {
            // Its own namespace: fine.
            ctx.persist(ModuleOp {
                ns: "raider".into(),
                op: ModuleWrite::Del { key: "mine".into() },
            });
            // The vault's: refused, boot or not.
            ctx.persist(ModuleOp {
                ns: "vault".into(),
                op: ModuleWrite::Del {
                    key: "not-mine".into(),
                },
            });
            Ok(())
        }
    }

    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(VaultModule), Box::new(BootRaider)],
    )
    .expect("two modules compose");
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(std::collections::HashMap::new(), &mut out, &mut fx)
        .expect("a refused write must not fail the boot");
    let ops = engine.take_module_ops();
    assert_eq!(ops.len(), 1, "only the module's own boot write lands");
    assert_eq!(ops[0].ns, "raider");
    assert_eq!(
        engine.world.refused_namespace_writes(),
        vec![("raider".into(), "vault".into(), Some("vault".into()))]
    );
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
    // RulesetStateStore in RAM (the map blob went with S4: a map is the
    // atlas's window, never a scene's opaque document).
    assert_eq!(engine.world.scene_state(id, nick), Some(&json!({"hp": 10})));
    // Persisted as module ops: the state snapshot.
    let ops = engine.take_module_ops();
    assert_eq!(
        ops,
        vec![ModuleOp {
            ns: "toy".into(),
            op: ModuleWrite::JsonSet {
                key: SmolStr::new(format!("scene:{}:state", id.0)),
                value: json!({ nick.to_string(): {"hp": 10} }),
            },
        }]
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
        vec![ModuleOp {
            ns: "toy".into(),
            op: ModuleWrite::Del {
                key: SmolStr::new(format!("scene:{}:state", id.0)),
            },
        },]
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
        typed_state: mush_core::ModuleActionState::unavailable(),
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
    assert_eq!(
        dels.len(),
        1,
        "the scene's state document (the map went with S4)"
    );
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
        typed_state: mush_core::ModuleActionState::unavailable(),
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
            ..Default::default()
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

// ----------------------------------------------------------------------
// J1: ledger chunks reach a module as text, whichever way they came
// ----------------------------------------------------------------------

/// A rig that puts `log:` documents in `kv` (as every pre-J1 rig did) finds
/// them moved to `ledger`, serialized; a malformed `log:` key moves too (it is
/// the inspection's breach to report, not a stray document); and the move is
/// idempotent.
#[test]
fn j1_boot_data_normalizes_ledger_keys_to_text() {
    let chunk = json!({ "chunk": 0, "sealed": false, "entries": [{ "seq": 0, "value": 7 }] });
    let mut data = ModuleBootData {
        kv: vec![
            (SmolStr::new("cfg"), json!({ "a": 1 })),
            (SmolStr::new("log:s:0"), chunk.clone()),
            (SmolStr::new("log:not-a-chunk"), json!(1)),
        ],
        ..Default::default()
    };
    data.normalize();
    assert_eq!(data.kv, vec![(SmolStr::new("cfg"), json!({ "a": 1 }))]);
    assert_eq!(data.ledger.len(), 2);
    assert_eq!(data.ledger[0].0, "log:s:0");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&data.ledger[0].1).unwrap(),
        chunk
    );
    assert_eq!(
        data.ledger[1],
        (SmolStr::new("log:not-a-chunk"), "1".to_string())
    );
    data.normalize();
    assert_eq!(data.ledger.len(), 2, "idempotent");
    assert_eq!(data.fetch_ms, 0, "a rig measures no fetch");
}

// ----------------------------------------------------------------------
// fu3-4: a global verb may not capture a behavior verb
// ----------------------------------------------------------------------
//
// A behavior's contextual commands are consulted only after the global
// command table misses (`cmd.rs`: the behavior stage runs on
// `LookupErr::NotFound`). So a global verb that resolves the same word wins
// forever and the behavior verb is dead with nothing logged. `register_behavior`
// refuses the forward order at registration; `Engine::compose_modules` refuses
// BOTH orders at the composition freeze point, which is the order-independent
// half.

fn shadow_noop(_ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    Ok(())
}

fn shadow_noop_behavior(_ctx: &mut Ctx<'_>, _target: &BehaviorTarget) -> Result<(), CmdError> {
    Ok(())
}

/// A module owning one behavior that declares `verbs` as contextual commands.
struct ShadowBehaviorMod {
    name: &'static str,
    id: &'static str,
    verbs: Vec<&'static str>,
}

impl GameModule for ShadowBehaviorMod {
    fn name(&self) -> &'static str {
        self.name
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let mut spec = BehaviorSpec::new(BehaviorId::new(self.id).expect("valid behavior id"));
        for verb in &self.verbs {
            spec = spec.with_command(BehaviorCommand::new(
                CommandSpec {
                    name: (*verb).into(),
                    shape: ArgShape::None,
                    ..CommandSpec::default()
                },
                Arc::new(shadow_noop_behavior),
            ));
        }
        reg.behavior(spec)
    }
}

/// A module owning `verbs` in the GLOBAL command table.
struct ShadowGlobalMod {
    name: &'static str,
    verbs: Vec<&'static str>,
}

impl GameModule for ShadowGlobalMod {
    fn name(&self) -> &'static str {
        self.name
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
        for verb in &self.verbs {
            reg.command(
                CommandSpec {
                    name: (*verb).into(),
                    shape: ArgShape::None,
                    ..CommandSpec::default()
                },
                Arc::new(shadow_noop),
            )?;
        }
        Ok(())
    }
}

fn turret(verbs: Vec<&'static str>) -> Box<dyn GameModule> {
    Box::new(ShadowBehaviorMod {
        name: "gunnery",
        id: "test.turret",
        verbs,
    })
}

fn sprinklers(verbs: Vec<&'static str>) -> Box<dyn GameModule> {
    Box::new(ShadowGlobalMod {
        name: "sprinklers",
        verbs,
    })
}

/// The reverse order `register_behavior` cannot see: the behavior composes
/// first (nothing to collide with yet), then a later module registers the same
/// bare global. Before the freeze-point pass this composed cleanly and the
/// behavior's `fire` was permanently unreachable with no log line.
#[test]
fn a_global_verb_composed_after_a_behavior_verb_is_refused() {
    let error = Engine::compose_modules(
        Registry::new(),
        vec![turret(vec!["fire"]), sprinklers(vec!["fire"])],
    )
    .err()
    .expect("a global capturing a behavior verb must fail composition");
    let text = error.to_string();
    assert!(text.contains("shadowed"), "{text}");
    assert!(text.contains("\"fire\""), "{text}");
    assert!(text.contains("gunnery"), "the behavior's owner: {text}");
    assert!(
        text.contains("sprinklers"),
        "the capturing global's owner: {text}"
    );
}

/// The property the `register_tagged` placement could not have had: the same
/// roster composed the other way round is refused too, naming the same verb and
/// the same two modules. (This direction trips `register_behavior`'s earlier,
/// better-localised diagnostic first — the refusal, not its wording, is what is
/// order-independent.)
#[test]
fn the_reverse_module_order_is_refused_identically() {
    let error = Engine::compose_modules(
        Registry::new(),
        vec![sprinklers(vec!["fire"]), turret(vec!["fire"])],
    )
    .err()
    .expect("the reverse order must fail composition too");
    let text = error.to_string();
    assert!(text.contains("fire"), "{text}");
    assert!(text.contains("gunnery"), "the behavior's owner: {text}");
    assert!(
        text.contains("sprinklers"),
        "the capturing global's owner: {text}"
    );
}

/// Dispatch resolves a bare verb by unique prefix, so a longer global name eats
/// a shorter behavior verb just as an exact hit would. The pass asks
/// `lookup_idx`, the same resolver, so this falls out rather than being
/// re-derived.
#[test]
fn a_global_name_that_prefix_captures_a_behavior_verb_is_refused() {
    let error = Engine::compose_modules(
        Registry::new(),
        vec![turret(vec!["fire"]), sprinklers(vec!["firewall"])],
    )
    .err()
    .expect("a prefix-capturing global must fail composition");
    let text = error.to_string();
    assert!(text.contains("shadowed"), "{text}");
    assert!(text.contains("firewall"), "{text}");
}

/// `@`-prefixed globals are exempt from prefix matching (`lookup_idx` refuses to
/// prefix-match a name starting with `@` or `+`), so `@fire` never reaches a
/// bare `fire` and the roster composes — in either order.
#[test]
fn an_at_prefixed_global_does_not_shadow_a_bare_behavior_verb() {
    Engine::compose_modules(
        Registry::new(),
        vec![turret(vec!["fire"]), sprinklers(vec!["@fire"])],
    )
    .expect("an @-prefixed global cannot capture a bare behavior verb");
    Engine::compose_modules(
        Registry::new(),
        vec![sprinklers(vec!["@fire"]), turret(vec!["fire"])],
    )
    .expect("nor in the other composition order");
}

/// Two bare globals sharing the behavior verb as a prefix make the verb
/// AMBIGUOUS at dispatch — the global stage refuses it and never falls through
/// to the behavior stage, so the behavior command is just as dead. The
/// diagnostic lists the candidates.
#[test]
fn an_ambiguous_prefix_is_refused_with_the_candidate_list() {
    let error = Engine::compose_modules(
        Registry::new(),
        vec![
            turret(vec!["fire"]),
            sprinklers(vec!["firewall", "firearm"]),
        ],
    )
    .err()
    .expect("an ambiguous global prefix must fail composition");
    let text = error.to_string();
    assert!(text.contains("ambiguous"), "{text}");
    assert!(
        text.contains("firewall") && text.contains("firearm"),
        "{text}"
    );
}

// ----------------------------------------------------------------------
// The typed-state lens (WS03-S7)
// ----------------------------------------------------------------------

/// A `Perceive` hook that answers PURELY from whether its read context can
/// reach live typed state: it conceals when it cannot.
///
/// That is the inverse of a real ruleset hook, and deliberately so. A hook
/// that reads a typed family (WoD's Obfuscate cache is the live example)
/// abstains — the wrong answer, silently — when the context has no store. The
/// old API let a production caller reach that state by typing the *shorter*
/// name: `can_perceive` passed `None`, its longer twin passed the store, and
/// nothing made a caller choose. This probe turns that silent wrong answer
/// into a loud one: a store-less call on any of the three production paths
/// below hides Bob outright.
struct TypedStateProbeHook;

impl EventHook for TypedStateProbeHook {
    fn perceives(
        &self,
        ctx: &HookReadContext<'_>,
        _observer: ObjId,
        _subject: ObjId,
    ) -> mush_core::module::Perceive {
        if ctx.typed_state_available() {
            mush_core::module::Perceive::Abstain
        } else {
            mush_core::module::Perceive::Conceal
        }
    }

    fn is_concealed(&self, ctx: &HookReadContext<'_>, _subject: ObjId) -> bool {
        !ctx.typed_state_available()
    }
}

/// Composes the probe hook plus one command that reports what the *`Ctx`*
/// surface sees, so the module-facing predicates are covered as well as the
/// engine-facing ones.
struct ProbeModule {
    observer: ObjId,
    subject: ObjId,
}

impl GameModule for ProbeModule {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
        reg.hook(HookPoint::Perceive, Arc::new(TypedStateProbeHook));
        let (observer, subject) = (self.observer, self.subject);
        reg.command(
            CommandSpec {
                name: "+probe".into(),
                shape: ArgShape::None,
                help: "+probe -- report the Ctx perception surface.".into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let perceives = ctx.can_perceive(observer, subject);
                let concealed = ctx.is_concealed(subject);
                let actor = ctx.actor;
                ctx.out.info(
                    actor,
                    format!("perceives={perceives} concealed={concealed}"),
                );
                Ok(())
            }),
        )
    }
}

/// Every room name a `look`-shaped outbox rendered, with its contents.
fn room_contents(out: &Outbox) -> Vec<(String, Vec<String>)> {
    out.events()
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Room(rv) => Some((
                rv.name.clone(),
                rv.contents.iter().map(|r| r.name.clone()).collect(),
            )),
            _ => None,
        })
        .collect()
}

/// WS03-S7. The perception gate must see the LIVE typed-state snapshot on
/// every path production can reach it by — the `look` render, the arrival
/// render a move produces, and the `Ctx` predicates a module calls — and a
/// deliberately detached lens must still give the store-less answer.
///
/// Before the collapse each of those paths had two spellings, and the
/// store-less one was the shorter, more reachable name. This test would have
/// caught any of them regressing to it: the probe conceals Bob the moment a
/// context loses its store, and concealment is the *wrong perception answer*,
/// not a degraded one.
#[test]
fn a_perceive_hook_sees_live_typed_state_on_every_production_path() {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let hall = world.create(ObjKind::Room, "Hall", GOD, None).unwrap();
    let vault = world.create(ObjKind::Room, "Vault", GOD, None).unwrap();
    let alice = world
        .create(ObjKind::Player, "Alice", GOD, Some(hall))
        .unwrap();
    let bob = world
        .create(ObjKind::Player, "Bob", GOD, Some(hall))
        .unwrap();
    let carol = world
        .create(ObjKind::Player, "Carol", GOD, Some(vault))
        .unwrap();
    for who in [alice, bob, carol] {
        world.set_connected(who, true);
    }
    let door = world
        .create(ObjKind::Exit, "north;n", GOD, Some(hall))
        .unwrap();
    world.link_exit(door, Some(vault)).unwrap();

    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    let mut engine = Engine::with_modules(
        world,
        registry,
        vec![Box::new(ProbeModule {
            observer: alice,
            subject: bob,
        })],
    )
    .unwrap();

    // Path 1 — the `look` room render (output::room_view under Ctx::show_room).
    let out = run(&mut engine, alice, "look");
    let rendered = room_contents(&out);
    assert!(
        rendered
            .iter()
            .any(|(room, contents)| room == "Hall" && contents.iter().any(|name| name == "Bob")),
        "the look render must carry the live store, so the probe abstains: {rendered:?}"
    );

    // Path 2 — the per-target examine gate (commands::look's can_perceive).
    let out = run(&mut engine, alice, "look Bob");
    assert!(
        !sys_texts(&out)
            .iter()
            .any(|line| line.contains("don't see")),
        "looking at a visible player must not be refused: {:?}",
        sys_texts(&out)
    );

    // Path 3 — the module-facing `Ctx` predicates.
    let out = run(&mut engine, alice, "+probe");
    assert_eq!(
        sys_texts(&out),
        vec!["perceives=true concealed=false".to_string()],
        "Ctx::can_perceive / Ctx::is_concealed must read the command's live lens"
    );

    // Path 4 — the arrival render a move produces (movement::traverse_exit).
    let out = run(&mut engine, alice, "north");
    let rendered = room_contents(&out);
    assert!(
        rendered
            .iter()
            .any(|(room, contents)| room == "Vault" && contents.iter().any(|name| name == "Carol")),
        "the arrival render must carry the live store too: {rendered:?}"
    );

    // And the other side of the lens: an explicitly detached call still gets
    // the store-less verdict, which is what makes the paths above meaningful.
    assert!(
        mush_core::can_perceive(
            &engine.registry,
            &engine.world,
            alice,
            bob,
            mush_core::TypedStateLens::Live(engine.action_state()),
        ),
        "a live lens abstains"
    );
    assert!(
        !mush_core::can_perceive(
            &engine.registry,
            &engine.world,
            alice,
            bob,
            mush_core::TypedStateLens::Detached(mush_core::DetachedTypedState::Test),
        ),
        "a detached lens conceals — the answer the store-less twin used to hand out"
    );
    assert!(
        mush_core::is_concealed(
            &engine.registry,
            &engine.world,
            bob,
            mush_core::TypedStateLens::Detached(mush_core::DetachedTypedState::Test),
        ),
        "and the notability query answers the same way"
    );
}

// ----------------------------------------------------------------------
// The async-completion path (WS03-S13)
// ----------------------------------------------------------------------

/// The typed value the async probe below reads back.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WardState {
    charges: u8,
}

type WardHandle = mush_core::StateFamilyHandle<u64, WardState>;

/// A module whose [`AsyncCompletion`] handler READS a typed family and reports
/// what it saw.
///
/// `apply_async_completion` was one of the fourteen twins WS03-S13 collapsed:
/// the store-less spelling was the shorter, more reachable name, so a caller
/// that dropped the store handed the handler a `Ctx` whose family read failed
/// outright — not a stale value, no value at all. This module makes that
/// failure observable: the handler prints the charge count it read, or the
/// error it got instead.
struct WardModule {
    handle: Arc<std::sync::Mutex<Option<WardHandle>>>,
    completion: Arc<WardCompletion>,
}

struct WardCompletion {
    handle: Arc<std::sync::Mutex<Option<WardHandle>>>,
}

impl mush_core::module::AsyncCompletion for WardCompletion {
    fn on_complete(&self, ctx: &mut Ctx<'_>, correlation: u64, _payload: &str) {
        let guard = self.handle.lock().expect("ward handle mutex");
        let handle = guard.as_ref().expect("the family registered at compose");
        let line = match ctx.action_state(handle, &correlation) {
            Ok(Some(state)) => format!("charges={}", state.charges),
            Ok(None) => "no-record".to_string(),
            Err(error) => format!("unreadable: {error}"),
        };
        let actor = ctx.actor;
        ctx.out.info(actor, line);
    }
}

impl GameModule for WardModule {
    fn name(&self) -> &'static str {
        "ward"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
        let descriptor = mush_core::StateFamilyDescriptor::new(
            mush_core::StateFamilyId::new("wards").expect("static family id"),
            CanonicalId::new("u64.decimal.v1").expect("static codec id"),
            CanonicalId::new("ward_state.v1").expect("static schema id"),
            1,
            CanonicalId::new("ward_state.decode_json.v1").expect("static decoder id"),
            CanonicalId::new("ward_state.range.v1").expect("static validator id"),
            CanonicalId::new("ward_state.json.v1").expect("static serializer id"),
        )
        .expect("static state-family descriptor");
        let handle = reg.state_family(
            descriptor,
            |key: &u64| Ok(key.to_string()),
            |value: &serde_json::Value| {
                serde_json::from_value(value.clone()).map_err(|error| error.to_string())
            },
            |_value: &WardState| Ok(()),
            |value: &WardState| Ok(json!({ "charges": value.charges })),
        )?;
        *self.handle.lock().expect("ward handle mutex") = Some(handle);
        reg.async_completion(
            Arc::clone(&self.completion) as Arc<dyn mush_core::module::AsyncCompletion>
        )
    }
}

/// WS03-S13. The async-completion route is a production dispatch path: the
/// engine routes [`WorldCommand::ModuleAsyncDone`] to the module's handler with
/// a `Ctx`, and that `Ctx` must carry the LIVE typed-state snapshot. It used to
/// have two spellings — `apply_async_completion` passing `None` beside a longer
/// twin passing the store — and the short one was the easy one to reach.
///
/// The probe reads a seeded typed family, so the difference is not cosmetic: on
/// the engine's own path it reads `charges=3`; called with an explicitly
/// detached lens it cannot read the family at all.
#[test]
fn an_async_completion_hook_sees_live_typed_state() {
    let handle_cell: Arc<std::sync::Mutex<Option<WardHandle>>> = Arc::default();
    let composition = Engine::compose_modules(
        Registry::new(),
        vec![Box::new(WardModule {
            handle: Arc::clone(&handle_cell),
            completion: Arc::new(WardCompletion {
                handle: Arc::clone(&handle_cell),
            }),
        })],
    )
    .expect("the ward module composes");
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let mut engine = Engine::from_composition(world, composition);
    let handle = handle_cell
        .lock()
        .expect("ward handle mutex")
        .clone()
        .expect("composition registered the family");

    // Seed one live record under correlation 7 through the public transaction
    // surface, so the read below has something real to find.
    let installation = {
        let mut txn = engine
            .action_transaction(&handle)
            .expect("the ward family opens a transaction");
        txn.insert(&handle, &7, WardState { charges: 3 })
            .expect("the seed insert is accepted");
        let (_, installation) = txn
            .prepare()
            .expect("the seed transaction prepares")
            .into_parts();
        installation
    };
    engine
        .install_prepared_action(installation)
        .expect("the seed installs");

    // The production path: the engine's own `ModuleAsyncDone` routing.
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::Apply(mush_core::WorldCommand::ModuleAsyncDone {
            module: SmolStr::new("ward"),
            actor: GOD,
            correlation: 7,
            payload: "{}".into(),
        }),
        &mut out,
        &mut fx,
    );
    assert_eq!(
        sys_texts(&out),
        vec!["charges=3".to_string()],
        "the engine's async-completion routing must hand the handler the live store"
    );

    // And the other side of the lens: named detachment, and only named
    // detachment, takes the family away.
    let mut out = Outbox::new();
    mush_core::module::apply_async_completion(
        &engine.registry,
        &mut engine.world,
        &mut out,
        SmolStr::new("ward"),
        GOD,
        7,
        "{}",
        mush_core::TypedStateLens::Detached(mush_core::DetachedTypedState::Test),
    );
    let detached = sys_texts(&out);
    assert!(
        detached.len() == 1 && detached[0].starts_with("unreadable:"),
        "a detached lens must leave the family unreadable, not silently empty: {detached:?}"
    );
}

// ----------------------------------------------------------------------
// 16b cut U.4 (16c `B-37`): the post-boot hook
// ----------------------------------------------------------------------

/// A module that writes what it was called for into a shared log.
struct BootProbe {
    name: &'static str,
    log: Arc<std::sync::Mutex<Vec<String>>>,
    refuse: bool,
}

impl GameModule for BootProbe {
    fn name(&self) -> &'static str {
        self.name
    }

    fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), ModError> {
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        _ctx: &mut ModuleCtx<'_>,
        _data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.log
            .lock()
            .expect("the log")
            .push(format!("loaded {}", self.name));
        Ok(())
    }

    fn on_all_loaded(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        // The same identity a load gets: this module's name, acting as GOD.
        let who = format!(
            "all {} as {} by {}",
            self.name,
            ctx.module.as_deref().unwrap_or("<none>"),
            ctx.actor
        );
        self.log.lock().expect("the log").push(who);
        if self.refuse {
            return Err(ModError::msg(format!("{} refuses the boot", self.name)));
        }
        Ok(())
    }
}

fn probes(
    log: &Arc<std::sync::Mutex<Vec<String>>>,
    refusing: Option<&'static str>,
) -> Vec<Box<dyn GameModule>> {
    ["alpha", "bravo", "charlie"]
        .into_iter()
        .map(|name| {
            Box::new(BootProbe {
                name,
                log: Arc::clone(log),
                refuse: refusing == Some(name),
            }) as Box<dyn GameModule>
        })
        .collect()
}

/// **`on_all_loaded` runs once per module, in list order, after every module's
/// `on_world_loaded`** — the hook a module needs for state a module composed after
/// it produces (the tacsim pod sweep and the content latch) — **and its error
/// fails the boot**, as a load's does, with the modules after it never called.
#[test]
fn on_all_loaded_runs_once_each_in_list_order_after_every_load_and_can_fail_the_boot() {
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut engine =
        Engine::with_modules(World::new_minimal(), Registry::new(), probes(&log, None))
            .expect("three probes compose");
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(std::collections::HashMap::new(), &mut out, &mut fx)
        .expect("boot");
    assert_eq!(
        log.lock().expect("the log").clone(),
        vec![
            "loaded alpha".to_owned(),
            "loaded bravo".to_owned(),
            "loaded charlie".to_owned(),
            format!("all alpha as alpha by {GOD}"),
            format!("all bravo as bravo by {GOD}"),
            format!("all charlie as charlie by {GOD}"),
        ],
        "every load first, then every post-boot hook once, each in list order"
    );

    // The J4 shape: the driver delivers the modules one at a time and ends the
    // boot itself. The hook is the same call, so it is the same log.
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut engine =
        Engine::with_modules(World::new_minimal(), Registry::new(), probes(&log, None))
            .expect("three probes compose");
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    for name in ["alpha", "bravo", "charlie"] {
        engine
            .module_loaded(name, ModuleBootData::default(), &mut out, &mut fx)
            .expect("one module loads");
    }
    assert_eq!(
        log.lock().expect("the log").len(),
        3,
        "module_loaded alone never runs the post-boot hook"
    );
    engine
        .all_modules_loaded(&mut out, &mut fx)
        .expect("the end of the boot");
    assert_eq!(log.lock().expect("the log").len(), 6);

    // A refusal fails the boot and stops the walk where it stood.
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        probes(&log, Some("bravo")),
    )
    .expect("three probes compose");
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    let refused = engine
        .modules_loaded(std::collections::HashMap::new(), &mut out, &mut fx)
        .expect_err("a post-boot hook's error fails the boot");
    assert!(
        refused.to_string().contains("bravo refuses the boot"),
        "{refused}"
    );
    let seen = log.lock().expect("the log").clone();
    assert_eq!(
        seen.last().map(String::as_str),
        Some(format!("all bravo as bravo by {GOD}").as_str()),
        "the modules after the refusal are not called: {seen:?}"
    );
    assert!(!seen.iter().any(|line| line.starts_with("all charlie")));
}
