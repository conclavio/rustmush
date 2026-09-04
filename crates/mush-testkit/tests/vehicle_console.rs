//! Cut 2 proof (design 06 §4.3–4.4, the Phase A exit criterion): a toy
//! `vehicle.console` behavior contributes contextual commands that exist only
//! for an actor who can reach a console — carried, as a cockpit container,
//! ambient in the room (opt-in), or operated through a published
//! command-subject link. Drives the real `Engine` through `TestWorld`,
//! including link republish on reboot and cleanup on destroy.

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::{
    ArgShape, Attr, Audience, BehaviorCommand, BehaviorId, BehaviorSpec, BehaviorTarget, CmdError,
    CommandReach, CommandSpec, CommandSubjectLink, Ctx, GOD, GameModule, LIMBO, ModError,
    ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, ObjId, ObjKind, Registrar, Role, WorldEvent,
    WorldEventKind, match_object,
};
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use smol_str::SmolStr;

const NS: &str = "vehicle";
const CONSOLE: &str = "vehicle.console";

#[derive(Clone, Debug, Default)]
struct ConsoleState {
    /// actor → operated console: the authoritative assignment. The World's
    /// link index is a rebuildable projection of this map.
    pilots: HashMap<ObjId, ObjId>,
}

fn console_id() -> BehaviorId {
    BehaviorId::new(CONSOLE).unwrap()
}

fn console_spec() -> BehaviorSpec {
    let mut s = BehaviorSpec::new(console_id());
    s.allowed_kinds = vec![ObjKind::Thing];
    s.help = "A vehicle control console.".into();
    s.commands = vec![
        BehaviorCommand::new(
            CommandSpec {
                name: "startup".into(),
                shape: ArgShape::None,
                help: "startup -- power the console up.".into(),
                ..CommandSpec::default()
            },
            Arc::new(startup),
        ),
        BehaviorCommand::new(
            CommandSpec {
                name: "throttle".into(),
                switches: vec!["full".into(), "idle".into()],
                shape: ArgShape::Raw,
                help: "throttle/full | throttle/idle | throttle <n>".into(),
                ..CommandSpec::default()
            },
            Arc::new(throttle),
        ),
        BehaviorCommand::new(
            CommandSpec {
                name: "beacon".into(),
                shape: ArgShape::None,
                help: "beacon -- ping the console.".into(),
                ..CommandSpec::default()
            },
            Arc::new(beacon),
        )
        .with_reach(vec![CommandReach::Ambient, CommandReach::Carried]),
    ];
    s
}

fn name_of(ctx: &Ctx<'_>, id: ObjId) -> String {
    ctx.w
        .get(id)
        .map_or_else(|| id.to_string(), |o| o.name.clone())
}

fn power_on(ctx: &Ctx<'_>, id: ObjId) -> bool {
    ctx.w.attr(id, "vehicle:power").and_then(|a| a.as_text()) == Some("on")
}

fn startup(ctx: &mut Ctx<'_>, t: &BehaviorTarget) -> Result<(), CmdError> {
    let (subject, actor) = (t.subject, ctx.actor);
    if power_on(ctx, subject) {
        return Err(CmdError::msg(format!(
            "{} is already powered up.",
            name_of(ctx, subject)
        )));
    }
    ctx.w
        .set_attr(subject, "vehicle:power", Attr::text("on", actor))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    let n = name_of(ctx, subject);
    ctx.out.info(actor, format!("You power up {n}."));
    Ok(())
}

fn throttle(ctx: &mut Ctx<'_>, t: &BehaviorTarget) -> Result<(), CmdError> {
    let (subject, actor) = (t.subject, ctx.actor);
    if !power_on(ctx, subject) {
        return Err(CmdError::msg("Power it up first."));
    }
    let setting = if ctx.switches.has("full") {
        "full".to_string()
    } else if ctx.switches.has("idle") {
        "idle".to_string()
    } else {
        let r = ctx.args.raw().trim().to_string();
        if r.is_empty() {
            return Err(CmdError::msg("Throttle to what?"));
        }
        r
    };
    ctx.w
        .set_attr(
            subject,
            "vehicle:throttle",
            Attr::text(setting.clone(), actor),
        )
        .map_err(|e| CmdError::msg(e.to_string()))?;
    ctx.out.info(actor, format!("Throttle set to {setting}."));
    Ok(())
}

fn beacon(ctx: &mut Ctx<'_>, t: &BehaviorTarget) -> Result<(), CmdError> {
    let actor = ctx.actor;
    ctx.w
        .set_attr(t.subject, "vehicle:beacon", Attr::text("1", actor))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    let n = name_of(ctx, t.subject);
    ctx.out
        .info(actor, format!("The console at {n} chirps ({:?}).", t.reach));
    Ok(())
}

fn pilot_key(actor: ObjId) -> SmolStr {
    SmolStr::new(format!("pilot:{}", actor.0))
}

/// `+pilot <actor>=<console>` assigns; `+pilot <actor>=` clears. The module
/// persists its assignment and publishes the link; the link index is only a
/// projection, republished on boot.
fn pilot_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().unwrap_or("").trim().to_string();
    if rhs.is_empty() {
        if let Some(st) = ctx.state_mut::<ConsoleState>() {
            st.pilots.remove(&actor);
        }
        ctx.w.retract_command_subjects(NS, Some(actor));
        ctx.persist(ModuleOp {
            ns: SmolStr::new_static(NS),
            op: ModuleWrite::Del {
                key: pilot_key(actor),
            },
        });
        ctx.out.info(ctx.actor, "Pilot assignment cleared.");
        return Ok(());
    }
    let subject = match_object(ctx.w, ctx.actor, &rhs)?;
    if !ctx.w.has_behavior(subject, &console_id()) {
        return Err(CmdError::msg("That is not a console."));
    }
    if let Some(st) = ctx.state_mut::<ConsoleState>() {
        st.pilots.insert(actor, subject);
    }
    ctx.w.publish_command_subject(CommandSubjectLink {
        actor,
        subject,
        station: SmolStr::new_static("cockpit"),
        relation: SmolStr::new_static("pilot"),
        owner: SmolStr::new_static(NS),
        priority: 0,
        channels: Default::default(),
        epoch: 0,
    });
    ctx.persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op: ModuleWrite::JsonSet {
            key: pilot_key(actor),
            value: serde_json::json!({ "subject": subject.0 }),
        },
    });
    ctx.out.info(ctx.actor, "Pilot assigned.");
    Ok(())
}

/// Drop assignments touching a destroyed object. Core already scrubbed the
/// link index; this retires the authoritative record and its persisted doc.
fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    let obj = *obj;
    let dropped: Vec<ObjId> = ctx
        .state_mut::<ConsoleState>()
        .map(|st| {
            let d: Vec<ObjId> = st
                .pilots
                .iter()
                .filter(|(a, s)| **a == obj || **s == obj)
                .map(|(a, _)| *a)
                .collect();
            for a in &d {
                st.pilots.remove(a);
            }
            d
        })
        .unwrap_or_default();
    for a in dropped {
        ctx.w.retract_command_subjects(NS, Some(a));
        ctx.persist(ModuleOp {
            ns: SmolStr::new_static(NS),
            op: ModuleWrite::Del { key: pilot_key(a) },
        });
    }
}

#[derive(Debug, Default)]
struct ConsoleModule;

impl GameModule for ConsoleModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.attr_namespace(NS)?;
        reg.state(ConsoleState::default());
        reg.behavior(console_spec())?;
        reg.command(
            CommandSpec {
                name: "+pilot".into(),
                shape: ArgShape::EqPair,
                min_role: Role::Builder,
                help: "+pilot <actor>=<console>  --  assign; +pilot <actor>= clears.".into(),
                ..CommandSpec::default()
            },
            Arc::new(pilot_cmd),
        )?;
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(on_obj_destroyed));
        Ok(())
    }

    /// Rebuild the in-RAM assignment map and republish links from the persisted
    /// `pilot:*` docs — the link index is never persisted by core.
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        for (key, value) in data.kv {
            let Some(a) = key.strip_prefix("pilot:") else {
                continue;
            };
            let Some(actor) = a.parse::<u32>().ok().map(ObjId) else {
                continue;
            };
            let Some(subject) = value.get("subject").and_then(|v| v.as_u64()) else {
                continue;
            };
            let subject = ObjId(subject as u32);
            if let Some(st) = ctx.state_mut::<ConsoleState>() {
                st.pilots.insert(actor, subject);
            }
            ctx.w.publish_command_subject(CommandSubjectLink {
                actor,
                subject,
                station: SmolStr::new_static("cockpit"),
                relation: SmolStr::new_static("pilot"),
                owner: SmolStr::new_static(NS),
                priority: 0,
                channels: Default::default(),
                epoch: 0,
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![Box::new(ConsoleModule)])
}

fn texts(ev: &[(Audience, OutputEvent)]) -> String {
    ev.iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Sys(t) => Some(t.to_plain_text()),
            OutputEvent::Narrative { text, .. } => Some(text.to_plain_text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn attr(tw: &TestWorld, id: ObjId, name: &str) -> Option<String> {
    tw.world()
        .attr(id, name)
        .and_then(|a| a.as_text())
        .map(str::to_string)
}

/// A bound console Thing at `loc`, owned by GOD.
fn console(tw: &mut TestWorld, name: &str, loc: ObjId) -> ObjId {
    let c = tw
        .world_mut()
        .create(ObjKind::Thing, name, GOD, Some(loc))
        .unwrap();
    tw.world_mut()
        .attach_behavior(
            c,
            console_id(),
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
    c
}

// ---------------------------------------------------------------------
// (a) carried console: verb, switches, mutation lands on the SUBJECT
// ---------------------------------------------------------------------

#[test]
fn carried_console_powers_up_and_the_subject_is_what_mutates() {
    let mut tw = boot();
    let bob = tw.spawn_player("Bob");
    let c = console(&mut tw, "Console", bob);
    let _ = tw.drain_write_batch(); // discard setup dirt

    let t = texts(&tw.run(bob, "startup"));
    assert!(t.contains("You power up Console."), "{t}");
    assert_eq!(attr(&tw, c, "vehicle:power").as_deref(), Some("on"));
    let batch = tw.drain_write_batch();
    assert!(
        batch.dirty.contains(&c) && !batch.dirty.contains(&bob),
        "the console, not the actor, was dirtied: {:?}",
        batch.dirty
    );

    assert!(texts(&tw.run(bob, "startup")).contains("already powered up"));
    assert!(texts(&tw.run(bob, "throttle/full")).contains("Throttle set to full."));
    assert_eq!(attr(&tw, c, "vehicle:throttle").as_deref(), Some("full"));
    assert!(texts(&tw.run(bob, "throttle 42")).contains("Throttle set to 42."));
    // The spec's switch table is enforced exactly like a global command's.
    assert!(
        texts(&tw.run(bob, "throttle/warp")).contains("Unknown switch \"/warp\" for throttle.")
    );
}

// ---------------------------------------------------------------------
// (b) unreachable: none, unbound, or in someone else's inventory → Huh?
// ---------------------------------------------------------------------

#[test]
fn unreachable_consoles_leave_the_verb_undefined() {
    let mut tw = boot();
    let bob = tw.spawn_player("Bob");
    let ann = tw.spawn_player("Ann");
    assert!(
        texts(&tw.run(ann, "startup")).contains("Huh?"),
        "no console at all"
    );
    // An UNBOUND console in hand is just a thing.
    tw.world_mut()
        .create(ObjKind::Thing, "Dud", GOD, Some(ann))
        .unwrap();
    assert!(
        texts(&tw.run(ann, "startup")).contains("Huh?"),
        "unbound console"
    );
    // A bound console in BOB's inventory is not reachable by Ann.
    console(&mut tw, "Console", bob);
    assert!(
        texts(&tw.run(ann, "startup")).contains("Huh?"),
        "someone else's inventory"
    );
}

// ---------------------------------------------------------------------
// (c) two carried consoles are ambiguous, and nothing runs
// ---------------------------------------------------------------------

#[test]
fn two_carried_consoles_are_ambiguous_and_nothing_mutates() {
    let mut tw = boot();
    let bob = tw.spawn_player("Bob");
    let a = console(&mut tw, "Alpha", bob);
    let b = console(&mut tw, "Beta", bob);
    let t = texts(&tw.run(bob, "startup"));
    assert!(
        t.contains("Ambiguous") && t.contains("Alpha") && t.contains("Beta"),
        "{t}"
    );
    assert!(attr(&tw, a, "vehicle:power").is_none() && attr(&tw, b, "vehicle:power").is_none());
    tw.world_mut().detach_behavior(b, &console_id()).unwrap();
    assert!(texts(&tw.run(bob, "startup")).contains("You power up Alpha."));
}

// ---------------------------------------------------------------------
// (d) reach: ambient needs opt-in; a cockpit container beats a carried one
// ---------------------------------------------------------------------

#[test]
fn ambient_is_opt_in_and_a_container_outranks_carried() {
    let mut tw = boot();
    let bob = tw.spawn_player("Bob");
    // A console sitting in the room is not the room: no ambient reach at all.
    let floor = console(&mut tw, "Floor", LIMBO);
    assert!(texts(&tw.run(bob, "startup")).contains("Huh?"));
    assert!(texts(&tw.run(bob, "beacon")).contains("Huh?"));
    // Bind the ROOM: its opt-in `beacon` answers ambiently; `startup` still doesn't.
    tw.world_mut()
        .detach_behavior(floor, &console_id())
        .unwrap();
    tw.world_mut()
        .attach_behavior(
            LIMBO,
            console_id(),
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
    let t = texts(&tw.run(bob, "beacon"));
    assert!(t.contains("chirps (Ambient)"), "{t}");
    assert!(
        texts(&tw.run(bob, "startup")).contains("Huh?"),
        "startup has no ambient reach"
    );
    tw.world_mut()
        .detach_behavior(LIMBO, &console_id())
        .unwrap();

    // Bob climbs into a bound cockpit while also carrying a bound console: the
    // container tier wins outright (no ambiguity across tiers).
    let cockpit = console(&mut tw, "Cockpit", LIMBO);
    let hand = console(&mut tw, "Handheld", bob);
    tw.world_mut().move_to(bob, cockpit).unwrap();
    let t = texts(&tw.run(bob, "startup"));
    assert!(t.contains("You power up Cockpit."), "{t}");
    assert!(attr(&tw, hand, "vehicle:power").is_none());
}

// ---------------------------------------------------------------------
// (e) exits and global verbs always win; no prefix matching
// ---------------------------------------------------------------------

#[test]
fn exits_and_globals_beat_behavior_verbs_and_prefixes_do_not_reach() {
    let mut tw = boot();
    let bob = tw.spawn_player("Bob");
    let c = console(&mut tw, "Console", bob);
    assert!(
        texts(&tw.run(bob, "sta")).contains("Huh?"),
        "no prefix matching"
    );

    // An exit named `startup` in Bob's room wins (stage 5 precedes the stage).
    let hall = tw
        .world_mut()
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    let ex = tw
        .world_mut()
        .create(ObjKind::Exit, "startup", GOD, Some(LIMBO))
        .unwrap();
    tw.world_mut().link_exit(ex, Some(hall)).unwrap();
    tw.run(bob, "startup");
    assert_eq!(
        tw.world().get(bob).and_then(|o| o.location),
        Some(hall),
        "exit won"
    );
    assert!(attr(&tw, c, "vehicle:power").is_none());

    // A global verb registered later shadows it (global wins by design).
    tw.engine_mut()
        .registry
        .register(
            CommandSpec {
                name: "beacon".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| {
                ctx.out.info(ctx.actor, "global beacon");
                Ok(())
            }),
        )
        .unwrap();
    let t = texts(&tw.run(bob, "beacon"));
    assert!(t.contains("global beacon") && !t.contains("chirps"), "{t}");
}

// ---------------------------------------------------------------------
// (f) command-subject links: remote operation, persistence/reboot, cleanup
// ---------------------------------------------------------------------

#[test]
fn a_pilot_link_operates_a_remote_console_and_survives_reboot() {
    let mut tw = boot();
    let bob = tw.spawn_player("Bob");
    let mech = console(&mut tw, "Mech", LIMBO); // in the room, not carried
    assert!(
        texts(&tw.run(bob, "startup")).contains("Huh?"),
        "unlinked: unreachable"
    );

    let t = texts(&tw.run(GOD, &format!("+pilot *Bob={mech}")));
    assert!(t.contains("Pilot assigned."), "{t}");
    let t = texts(&tw.run(bob, "startup"));
    assert!(t.contains("You power up Mech."), "{t}");

    // Persist the assignment, reboot a fresh world at the same ids, republish.
    let ops = tw.take_module_ops();
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in ops.into_iter().filter(|o| o.ns == NS) {
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    assert!(
        kv.iter().any(|(k, _)| k.starts_with("pilot:")),
        "assignment persisted"
    );

    let mut tw2 = boot();
    let bob2 = tw2.spawn_player("Bob");
    let mech2 = console(&mut tw2, "Mech", LIMBO);
    assert_eq!((bob2, mech2), (bob, mech), "ids are dense and monotonic");
    assert!(
        texts(&tw2.run(bob2, "startup")).contains("Huh?"),
        "no link before republish"
    );
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new(NS),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
        },
    )]));
    assert!(
        texts(&tw2.run(bob2, "startup")).contains("You power up Mech."),
        "link republished"
    );

    // Clearing retracts it.
    tw2.run(GOD, "+pilot *Bob=");
    assert!(texts(&tw2.run(bob2, "startup")).contains("Huh?"));
}

#[test]
fn destroying_the_operated_object_retires_the_assignment() {
    let mut tw = boot();
    let bob = tw.spawn_player("Bob");
    let mech = console(&mut tw, "Mech", LIMBO);
    tw.run(GOD, &format!("+pilot *Bob={mech}"));
    let _ = tw.take_module_ops();
    assert!(texts(&tw.run(bob, "startup")).contains("You power up Mech."));

    let t = texts(&tw.run(GOD, &format!("@destroy {mech}")));
    assert!(t.contains("Destroyed."), "{t}");
    assert!(
        texts(&tw.run(bob, "startup")).contains("Huh?"),
        "link gone with the mech"
    );
    let ops = tw.take_module_ops();
    assert!(
        ops.iter()
            .any(|o| matches!(&o.op, ModuleWrite::Del { key } if key == &pilot_key(bob))),
        "observer retired the persisted assignment: {ops:?}"
    );
}
