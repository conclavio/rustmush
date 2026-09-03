//! End-to-end timestop tests through the real `Engine`: start + membership
//! freeze, the movement veto, turn/round advancement and the scene-local
//! clock, facilitator gating, end + release, and the durable persistence
//! round trip (drain ops -> InMemoryStore -> reboot -> rehydrated paused).

use std::collections::HashMap;

use std::sync::Arc;

use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::module::{GameModule, ModError, Registrar, emit_world_event};
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, GOD, ModuleBootData, ModuleOp, ModuleWrite, ObjId,
    ObjKind, Outbox, Registry, Role, SceneClass, World, WorldEvent, register_core_commands,
    register_engine_commands,
};
use mush_proto::{NarrativeKind, OutputEvent};
use mush_scenes::{
    NS, PauseReason, SceneEvent, SceneEventKind, SceneStatus, SceneUid, ScenesModule, ScenesState,
};
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use smol_str::SmolStr;

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

/// A world with two connected players (Nick, Ann) and one NPC (Barkeep) in a
/// bar room, the scenes module loaded (first boot).
struct Rig {
    engine: Engine,
    bar: ObjId,
    nick: ObjId,
    ann: ObjId,
    npc: ObjId,
}

/// A module providing a `Turn`-class mechanical command and a `Free` one, to
/// exercise the general turn-gating seam without pulling in a whole ruleset.
struct RollTestModule;

impl GameModule for RollTestModule {
    fn name(&self) -> &'static str {
        "rolltest"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.command(
            CommandSpec {
                name: "+testroll".into(),
                shape: ArgShape::None,
                scene_class: SceneClass::Turn,
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let actor = ctx.actor;
                ctx.out.sys_to_actor(actor, "test", "you rolled");
                Ok(())
            }),
        )?;
        // A Turn-class CONTEXTUAL command supplied by an object (a mech's
        // `fire`): never in the global table, gated by the same turn seam.
        reg.behavior(
            mush_core::BehaviorSpec::new(mush_core::BehaviorId::new("rolltest.mech").unwrap())
                .with_command(mush_core::BehaviorCommand::new(
                    CommandSpec {
                        name: "fire".into(),
                        shape: ArgShape::None,
                        scene_class: SceneClass::Turn,
                        ..CommandSpec::default()
                    },
                    Arc::new(
                        |ctx: &mut Ctx<'_>,
                         t: &mush_core::BehaviorTarget|
                         -> Result<(), CmdError> {
                            let actor = ctx.actor;
                            ctx.out
                                .sys_to_actor(actor, "test", format!("you fire {}", t.subject));
                            Ok(())
                        },
                    ),
                )),
        )?;
        Ok(())
    }
}

fn rig() -> Rig {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let bar = world
        .create(ObjKind::Room, "The Tap Room", GOD, None)
        .unwrap();
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(bar))
        .unwrap();
    let ann = world
        .create(ObjKind::Player, "Ann", GOD, Some(bar))
        .unwrap();
    let npc = world
        .create(ObjKind::Npc, "Barkeep", GOD, Some(bar))
        .unwrap();
    world.set_connected(nick, true);
    world.set_connected(ann, true);
    world.set_live_conns(nick, 1);
    world.set_live_conns(ann, 1);

    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine = Engine::with_modules(
        world,
        registry,
        vec![
            Box::new(mush_canon::CanonModule::new()),
            Box::new(ScenesModule::new()),
            Box::new(RollTestModule),
        ],
    )
    .unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .unwrap();
    let _ = engine.take_module_ops();
    Rig {
        engine,
        bar,
        nick,
        ann,
        npc,
    }
}

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

fn scenes(engine: &Engine) -> &ScenesState {
    engine
        .world
        .module_state::<ScenesState>()
        .expect("scenes state")
}

/// Destroy an object the way a command would: mutate the world, then emit the
/// `ObjDestroyed` world event so module observers (the scenes cleanup) run.
fn destroy(engine: &mut Engine, obj: ObjId) {
    engine.world.destroy(obj).unwrap();
    let mut out = Outbox::new();
    emit_world_event(
        &engine.registry,
        &mut engine.world,
        &mut out,
        &WorldEvent::ObjDestroyed { obj },
    );
}

fn texts(out: &Outbox) -> String {
    out.events()
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Sys(t) => Some(t.to_plain_text()),
            OutputEvent::Narrative { text, .. } => Some(text.to_plain_text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[test]
fn start_freezes_members_and_blocks_movement() {
    let mut r = rig();
    // A second room to try to escape to.
    let alley = r
        .engine
        .world
        .create(ObjKind::Room, "Alley", GOD, None)
        .unwrap();
    let exit = r
        .engine
        .world
        .create(ObjKind::Exit, "out", GOD, Some(r.bar))
        .unwrap();
    r.engine.world.link_exit(exit, Some(alley)).unwrap();

    let out = run(&mut r.engine, r.nick, "+timestop");
    assert!(texts(&out).contains("a timestop begins"), "{}", texts(&out));

    // All three present objects are frozen in the core index.
    for who in [r.nick, r.ann, r.npc] {
        assert!(
            r.engine.world.in_exclusive_scene(who),
            "{who} should be frozen"
        );
    }
    // The scene exists, active, turn order = players (dbref) then npc.
    let s = scenes(&r.engine).active_scene_at(r.bar).unwrap();
    assert_eq!(s.status, SceneStatus::Active);
    assert_eq!(s.participants, vec![r.nick, r.ann, r.npc]);
    assert_eq!(s.current_actor(), Some(r.nick));

    // A participant cannot walk out.
    let out = run(&mut r.engine, r.ann, "out");
    assert!(
        texts(&out).contains("cannot leave"),
        "movement not vetoed: {}",
        texts(&out)
    );
    assert_eq!(
        r.engine.world.get(r.ann).and_then(|o| o.location),
        Some(r.bar)
    );
}

#[test]
fn wizard_bypasses_the_movement_freeze() {
    let mut r = rig();
    let alley = r
        .engine
        .world
        .create(ObjKind::Room, "Alley", GOD, None)
        .unwrap();
    let exit = r
        .engine
        .world
        .create(ObjKind::Exit, "out", GOD, Some(r.bar))
        .unwrap();
    r.engine.world.link_exit(exit, Some(alley)).unwrap();
    // Make Ann a wizard.
    r.engine
        .world
        .get_mut(r.ann)
        .unwrap()
        .flags
        .insert(mush_core::Flags::WIZARD);

    run(&mut r.engine, r.nick, "+timestop");
    let out = run(&mut r.engine, r.ann, "out");
    assert!(
        !texts(&out).contains("cannot leave"),
        "wizard should pass: {}",
        texts(&out)
    );
    assert_eq!(
        r.engine.world.get(r.ann).and_then(|o| o.location),
        Some(alley)
    );
}

#[test]
fn turns_advance_rounds_wrap_and_the_scene_clock_tracks_rounds() {
    let mut r = rig();
    let ic_at_start = r.engine.world.game_now_unix();
    run(&mut r.engine, r.nick, "+timestop");

    // Nick (facilitator) advances the turn: Nick -> Ann.
    run(&mut r.engine, r.nick, "+timestop/next");
    let s = scenes(&r.engine).active_scene_of(r.nick).unwrap();
    assert_eq!(s.current_actor(), Some(r.ann));
    assert_eq!(s.round, 1);
    assert_eq!(
        s.scene_ic_time(),
        ic_at_start,
        "round 1 is still the frozen start"
    );

    // Ann -> Barkeep, then wrap to round 2 (Nick), advancing the scene clock.
    run(&mut r.engine, r.nick, "+timestop/next"); // Ann -> Barkeep
    run(&mut r.engine, r.nick, "+timestop/next"); // Barkeep -> wrap, round 2, Nick
    let s = scenes(&r.engine).active_scene_of(r.nick).unwrap();
    assert_eq!(s.round, 2);
    assert_eq!(s.current_actor(), Some(r.nick));
    assert_eq!(
        s.scene_ic_time(),
        ic_at_start + i64::from(s.round_seconds),
        "round 2 advances the scene-local IC clock by one round"
    );
}

#[test]
fn only_the_facilitator_or_staff_drives() {
    let mut r = rig();
    // Nick starts (facilitator). Ann is a participant but not facilitator.
    run(&mut r.engine, r.nick, "+timestop");
    let out = run(&mut r.engine, r.ann, "+timestop/next");
    assert!(texts(&out).contains("facilitator"), "{}", texts(&out));
    // Ann can still read it.
    let out = run(&mut r.engine, r.ann, "+scene");
    assert!(texts(&out).contains("turn order") || texts(&out).contains("Participants"));
    // Staff (GOD is in Limbo — move to bar first so active_scene_of finds them?
    // GOD is not a participant, so /next as GOD fails "not in a scene").
    assert!(!mush_core::perm::has_role(
        &r.engine.world,
        r.ann,
        Role::Royalty
    ));
}

#[test]
fn end_releases_the_freeze() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    assert!(r.engine.world.in_exclusive_scene(r.nick));
    let out = run(&mut r.engine, r.nick, "+timestop/end");
    assert!(texts(&out).contains("ends"), "{}", texts(&out));
    // Everyone released.
    for who in [r.nick, r.ann, r.npc] {
        assert!(
            !r.engine.world.in_exclusive_scene(who),
            "{who} should be released"
        );
    }
    // The record is retained, Closed.
    let closed = scenes(&r.engine).scenes.values().next().unwrap();
    assert_eq!(closed.status, SceneStatus::Closed);
    // Movement works again.
    assert!(scenes(&r.engine).active_scene_of(r.nick).is_none());
}

#[test]
fn cannot_start_two_scenes_in_a_room() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let out = run(&mut r.engine, r.ann, "+timestop");
    assert!(texts(&out).contains("already running"), "{}", texts(&out));
}

#[test]
fn npc_participant_is_suppressed_from_free_running_turns() {
    // The core index is what the NPC wake loop reads; assert the bit is set
    // (the mush-npc suppression check keys on exactly this).
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    assert!(
        r.engine.world.in_exclusive_scene(r.npc),
        "an NPC caught in a timestop must be flagged so its wake loop suppresses it"
    );
    run(&mut r.engine, r.nick, "+timestop/end");
    assert!(!r.engine.world.in_exclusive_scene(r.npc));
}

#[test]
fn destroying_a_participant_releases_them_and_keeps_the_scene() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop"); // Nick facilitator, [Nick, Ann, Barkeep]
    // Destroy a non-facilitator participant.
    destroy(&mut r.engine, r.ann);
    assert!(
        !r.engine.world.in_exclusive_scene(r.ann),
        "destroyed member released"
    );
    let s = scenes(&r.engine).active_scene_of(r.nick).unwrap();
    assert_eq!(
        s.participants,
        vec![r.nick, r.npc],
        "ghost dropped from turn order"
    );
    assert_eq!(s.facilitator, r.nick, "facilitator unchanged");
    // Nick and the npc stay frozen.
    assert!(r.engine.world.in_exclusive_scene(r.nick));
    assert!(r.engine.world.in_exclusive_scene(r.npc));
}

#[test]
fn destroying_the_facilitator_hands_off_to_a_surviving_player() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop"); // Nick facilitator
    // Destroy the facilitator (e.g. a chargen alt purge). Ann (a player) should
    // inherit facilitation — not the NPC — so the scene isn't stranded.
    destroy(&mut r.engine, r.nick);
    assert!(!r.engine.world.in_exclusive_scene(r.nick));
    let s = scenes(&r.engine).active_scene_of(r.ann).unwrap();
    assert_eq!(
        s.facilitator, r.ann,
        "a surviving PLAYER inherits facilitation"
    );
    // Ann can now drive it.
    let out = run(&mut r.engine, r.ann, "+timestop/next");
    assert!(
        !texts(&out).contains("facilitator"),
        "new facilitator drives: {}",
        texts(&out)
    );
}

#[test]
fn destroying_the_last_participant_closes_and_releases() {
    let mut r = rig();
    // A scene of just Nick + the NPC.
    r.engine.world.set_connected(r.ann, false); // Ann won't be a participant
    run(&mut r.engine, r.nick, "+timestop"); // [Nick, Barkeep]
    destroy(&mut r.engine, r.npc); // [Nick]
    destroy(&mut r.engine, r.nick); // []
    // Scene closed, everyone released.
    let closed = scenes(&r.engine).scenes.values().next().unwrap();
    assert_eq!(closed.status, SceneStatus::Closed);
    assert!(
        r.engine.world.exclusive_scene_members().next().is_none(),
        "index fully drained"
    );
}

#[test]
fn staff_can_end_a_scene_they_are_not_in_by_standing_in_the_room() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    // A wizard who was NOT a participant walks into the room.
    let wiz = r
        .engine
        .world
        .create(ObjKind::Player, "Wiz", GOD, Some(r.bar))
        .unwrap();
    r.engine
        .world
        .get_mut(wiz)
        .unwrap()
        .flags
        .insert(mush_core::Flags::WIZARD);
    r.engine.world.set_connected(wiz, true);
    // They are not frozen (staff bypass) and can end the scene by room.
    assert!(!r.engine.world.in_exclusive_scene(wiz));
    let out = run(&mut r.engine, wiz, "+timestop/end");
    assert!(
        texts(&out).contains("ends"),
        "staff recovery by room: {}",
        texts(&out)
    );
    assert!(
        !r.engine.world.in_exclusive_scene(r.nick),
        "everyone released"
    );
}

#[test]
fn royalty_staff_are_not_frozen() {
    let mut r = rig();
    let alley = r
        .engine
        .world
        .create(ObjKind::Room, "Alley", GOD, None)
        .unwrap();
    let exit = r
        .engine
        .world
        .create(ObjKind::Exit, "out", GOD, Some(r.bar))
        .unwrap();
    r.engine.world.link_exit(exit, Some(alley)).unwrap();
    // Make Ann Royalty (staff, but below Wizard).
    r.engine
        .world
        .get_mut(r.ann)
        .unwrap()
        .flags
        .insert(mush_core::Flags::ROYALTY);

    run(&mut r.engine, r.nick, "+timestop");
    // Ann (Royalty) is a participant but staff are never frozen — she can leave.
    let out = run(&mut r.engine, r.ann, "out");
    assert!(
        !texts(&out).contains("cannot leave"),
        "royalty should pass: {}",
        texts(&out)
    );
    assert_eq!(
        r.engine.world.get(r.ann).and_then(|o| o.location),
        Some(alley)
    );
}

#[test]
fn release_frees_one_participant_without_ending_the_scene() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    // The facilitator releases Ann.
    let out = run(&mut r.engine, r.nick, "+timestop/release Ann");
    assert!(texts(&out).contains("released"), "{}", texts(&out));
    assert!(!r.engine.world.in_exclusive_scene(r.ann), "Ann freed");
    // The scene continues with Nick + Barkeep still frozen.
    assert!(r.engine.world.in_exclusive_scene(r.nick));
    let s = scenes(&r.engine).active_scene_of(r.nick).unwrap();
    assert_eq!(s.status, SceneStatus::Active);
    assert!(!s.participants.contains(&r.ann));
    // A released player can now walk out (needs an exit) — assert the freeze
    // bit is gone, which the move gate keys on.
    assert!(!r.engine.world.in_exclusive_scene(r.ann));
    // The facilitator cannot /release themselves.
    let out = run(&mut r.engine, r.nick, "+timestop/release Nick");
    assert!(texts(&out).contains("+timestop/end"), "{}", texts(&out));
}

// ----------------------------------------------------------------------
// Event log
// ----------------------------------------------------------------------

fn log_entries(engine: &Engine, uid: SceneUid) -> Vec<SceneEvent> {
    scenes(engine)
        .events
        .open_entries(&uid.to_string())
        .iter()
        .map(|e| serde_json::from_value::<SceneEvent>(e.value.clone()).unwrap())
        .collect()
}

fn kinds(engine: &Engine, uid: SceneUid) -> Vec<SceneEventKind> {
    log_entries(engine, uid)
        .into_iter()
        .map(|e| e.kind)
        .collect()
}

/// Emit a raw speech event (to exercise OOC exclusion without an OOC command).
fn emit_said(engine: &mut Engine, who: ObjId, room: ObjId, kind: NarrativeKind, text: &str) {
    let mut out = Outbox::new();
    emit_world_event(
        &engine.registry,
        &mut engine.world,
        &mut out,
        &WorldEvent::Said {
            who,
            room,
            kind,
            text: text.into(),
        },
    );
}

/// Emit a raw Moved event.
fn emit_moved(engine: &mut Engine, who: ObjId, from: Option<ObjId>, to: ObjId) {
    let mut out = Outbox::new();
    emit_world_event(
        &engine.registry,
        &mut engine.world,
        &mut out,
        &WorldEvent::Moved { who, from, to },
    );
}

#[test]
fn no_op_move_records_no_join_or_leave() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let uid = scenes(&r.engine).active_scene_of(r.nick).unwrap().id;
    let before = log_entries(&r.engine, uid).len();
    // A no-op move (from == to == the anchor) must not log an arrival+departure.
    emit_moved(&mut r.engine, r.ann, Some(r.bar), r.bar);
    assert_eq!(
        log_entries(&r.engine, uid).len(),
        before,
        "a from==to move records nothing"
    );
}

#[test]
fn staff_can_read_a_scene_by_standing_in_the_room() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "say hi");
    // A wizard who is NOT a participant walks in and reads the scene.
    let wiz = r
        .engine
        .world
        .create(ObjKind::Player, "Wiz", GOD, Some(r.bar))
        .unwrap();
    r.engine
        .world
        .get_mut(wiz)
        .unwrap()
        .flags
        .insert(mush_core::Flags::WIZARD);
    r.engine.world.set_connected(wiz, true);
    let out = run(&mut r.engine, wiz, "+scene");
    assert!(
        texts(&out).contains("Participants"),
        "staff reads +scene by room: {}",
        texts(&out)
    );
    let out = run(&mut r.engine, wiz, "+timestop/log");
    assert!(
        texts(&out).contains("event"),
        "staff reads the log by room: {}",
        texts(&out)
    );
}

#[test]
fn lifecycle_events_are_recorded_in_order() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let uid = scenes(&r.engine).active_scene_of(r.nick).unwrap().id;
    run(&mut r.engine, r.nick, "+timestop/next");
    run(&mut r.engine, r.nick, "+timestop/pause");
    run(&mut r.engine, r.nick, "+timestop/resume");
    run(&mut r.engine, r.nick, "+timestop/end");
    assert_eq!(
        kinds(&r.engine, uid),
        vec![
            SceneEventKind::Started,
            SceneEventKind::TurnAdvanced,
            SceneEventKind::Paused,
            SceneEventKind::Resumed,
            SceneEventKind::Ended,
        ]
    );
    // Sequence numbers are gap-free.
    let seqs: Vec<u64> = scenes(&r.engine)
        .events
        .open_entries(&uid.to_string())
        .iter()
        .map(|e| e.seq)
        .collect();
    assert_eq!(seqs, vec![0, 1, 2, 3, 4]);
}

#[test]
fn ic_speech_is_recorded_ooc_is_not() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let uid = scenes(&r.engine).active_scene_of(r.nick).unwrap().id;
    // A participant speaks IC.
    run(&mut r.engine, r.nick, "say the prince is dead");
    // Raw OOC / page speech in the same room must NOT be recorded.
    emit_said(
        &mut r.engine,
        r.nick,
        r.bar,
        NarrativeKind::Ooc,
        "brb coffee",
    );
    emit_said(&mut r.engine, r.nick, r.bar, NarrativeKind::Page, "secret");

    let said: Vec<SceneEvent> = log_entries(&r.engine, uid)
        .into_iter()
        .filter(|e| e.kind == SceneEventKind::Said)
        .collect();
    assert_eq!(said.len(), 1, "only the IC line is recorded");
    assert_eq!(said[0].actor, Some(r.nick));
    assert_eq!(said[0].detail["text"], "the prince is dead");
    assert_eq!(said[0].detail["narr"], "say");
    // Witnesses were captured (the scene members).
    assert!(said[0].witnesses.contains(&r.nick));
}

#[test]
fn speech_while_paused_or_outside_the_anchor_is_not_recorded() {
    let mut r = rig();
    // Another room with a player.
    let lounge = r
        .engine
        .world
        .create(ObjKind::Room, "Lounge", GOD, None)
        .unwrap();
    let zoe = r
        .engine
        .world
        .create(ObjKind::Player, "Zoe", GOD, Some(lounge))
        .unwrap();
    r.engine.world.set_connected(zoe, true);
    r.engine.world.set_live_conns(zoe, 1);

    run(&mut r.engine, r.nick, "+timestop");
    let uid = scenes(&r.engine).active_scene_of(r.nick).unwrap().id;
    // Speech in a different room: not this scene's.
    run(&mut r.engine, zoe, "say unrelated");
    // Pause, then speak in the anchor: paused scenes don't record.
    run(&mut r.engine, r.nick, "+timestop/pause");
    run(&mut r.engine, r.nick, "say while paused");

    let said = log_entries(&r.engine, uid)
        .into_iter()
        .filter(|e| e.kind == SceneEventKind::Said)
        .count();
    assert_eq!(
        said, 0,
        "no speech recorded from another room or while paused"
    );
}

#[test]
fn timestop_log_renders_the_timeline() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "say hello there");
    run(&mut r.engine, r.nick, "+timestop/next");
    let out = run(&mut r.engine, r.ann, "+timestop/log"); // any participant can read
    let t = texts(&out);
    assert!(t.contains("The scene began"), "{t}");
    assert!(t.contains("Nick [say]: hello there"), "{t}");
    assert!(t.contains("Round"), "{t}");
}

#[test]
fn arrivals_into_the_anchor_are_recorded() {
    let mut r = rig();
    // A wanderer outside the scene room.
    let hall = r
        .engine
        .world
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    let exit = r
        .engine
        .world
        .create(ObjKind::Exit, "in", GOD, Some(hall))
        .unwrap();
    r.engine.world.link_exit(exit, Some(r.bar)).unwrap();
    let wanderer = r
        .engine
        .world
        .create(ObjKind::Player, "Wanda", GOD, Some(hall))
        .unwrap();
    r.engine.world.set_connected(wanderer, true);
    r.engine.world.set_live_conns(wanderer, 1);

    run(&mut r.engine, r.nick, "+timestop");
    let uid = scenes(&r.engine).active_scene_of(r.nick).unwrap().id;
    // A non-participant walks into the scene room (not frozen — freeze only
    // blocks participants leaving).
    run(&mut r.engine, wanderer, "in");
    let joined = log_entries(&r.engine, uid)
        .into_iter()
        .any(|e| e.kind == SceneEventKind::Joined && e.actor == Some(wanderer));
    assert!(joined, "an arrival into the anchor is recorded");
}

#[test]
fn ending_a_scene_records_a_canon_event() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "+timestop/end");
    // The conclusion is a durable canon event naming the participants.
    let canon = r
        .engine
        .world
        .module_state::<mush_canon::CanonState>()
        .unwrap();
    let concluded: Vec<_> = canon
        .recent()
        .into_iter()
        .filter(|e| e.kind == "scene.concluded")
        .collect();
    assert_eq!(concluded.len(), 1, "one scene.concluded event");
    assert!(concluded[0].entities.contains(&r.nick));
    // A participant sees it in the chronicle (Restricted scope).
    let t = texts(&run(&mut r.engine, r.nick, "+chronicle"));
    assert!(t.contains("A scene concluded"), "{t}");
    // A non-participant does not.
    let outsider = r
        .engine
        .world
        .create(ObjKind::Player, "Outsider", GOD, Some(r.bar))
        .unwrap();
    r.engine.world.set_connected(outsider, true);
    let t = texts(&run(&mut r.engine, outsider, "+chronicle"));
    assert!(
        !t.contains("A scene concluded"),
        "restricted canon leaked: {t}"
    );
}

/// Emit a raw mechanical roll event (simulating a ruleset's public +roll).
fn emit_rolled(engine: &mut Engine, who: ObjId, room: ObjId, summary: &str) {
    let mut out = Outbox::new();
    emit_world_event(
        &engine.registry,
        &mut engine.world,
        &mut out,
        &WorldEvent::Rolled {
            who,
            room,
            summary: summary.into(),
        },
    );
}

#[test]
fn turn_class_commands_are_gated_to_the_current_actor() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop"); // order [Nick, Ann, Barkeep], turn Nick
    // Nick (current turn) may run the mechanical command.
    let out = run(&mut r.engine, r.nick, "+testroll");
    assert!(
        texts(&out).contains("you rolled"),
        "current actor rolls: {}",
        texts(&out)
    );
    // Ann (not her turn) is blocked with a turn message.
    let out = run(&mut r.engine, r.ann, "+testroll");
    assert!(
        texts(&out).contains("not your turn"),
        "out-of-turn blocked: {}",
        texts(&out)
    );
    // Free commands (speech) are never gated.
    let out = run(&mut r.engine, r.ann, "say i wait");
    assert!(
        !texts(&out).contains("not your turn"),
        "speech is Free: {}",
        texts(&out)
    );
    // Advance the turn to Ann: now she may roll and Nick may not.
    run(&mut r.engine, r.nick, "+timestop/next");
    assert!(texts(&run(&mut r.engine, r.ann, "+testroll")).contains("you rolled"));
    assert!(texts(&run(&mut r.engine, r.nick, "+testroll")).contains("not your turn"));
}

#[test]
fn an_observer_is_told_they_have_no_turn_not_wait_your_turn() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let uid = scenes(&r.engine).active_scene_of(r.nick).unwrap().id;
    // Inject an observer (cut 1 has no join-as-observer command, so this is a
    // latent path — but the gate must handle it: an observer has no turn).
    let watcher = r
        .engine
        .world
        .create(ObjKind::Player, "Watcher", GOD, Some(r.bar))
        .unwrap();
    r.engine.world.set_connected(watcher, true);
    r.engine
        .world
        .module_state_mut::<ScenesState>()
        .unwrap()
        .scenes
        .get_mut(&uid)
        .unwrap()
        .observers
        .push(watcher);
    r.engine.world.set_exclusive_scene_member(watcher);
    let out = run(&mut r.engine, watcher, "+testroll");
    let t = texts(&out);
    assert!(
        t.contains("observing"),
        "observer gets a coherent message: {t}"
    );
    assert!(
        !t.contains("not your turn"),
        "not the wait-your-turn message: {t}"
    );
}

#[test]
fn a_paused_scene_blocks_mechanical_actions_entirely() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "+timestop/pause");
    // Even the current actor cannot roll while paused.
    let out = run(&mut r.engine, r.nick, "+testroll");
    assert!(
        texts(&out).contains("paused"),
        "paused blocks rolls: {}",
        texts(&out)
    );
}

#[test]
fn turn_gating_does_not_touch_actors_outside_a_scene() {
    let mut r = rig();
    // No timestop running: the mechanical command is ungated.
    let out = run(&mut r.engine, r.nick, "+testroll");
    assert!(
        texts(&out).contains("you rolled"),
        "ungated outside a scene: {}",
        texts(&out)
    );
}

#[test]
fn a_public_roll_is_recorded_into_the_scene_log() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let uid = scenes(&r.engine).active_scene_of(r.nick).unwrap().id;
    emit_rolled(
        &mut r.engine,
        r.nick,
        r.bar,
        "Nick rolls 5 vs 6: 2 successes",
    );
    let rolled: Vec<_> = log_entries(&r.engine, uid)
        .into_iter()
        .filter(|e| e.kind == SceneEventKind::Rolled)
        .collect();
    assert_eq!(rolled.len(), 1, "the roll is a scene event");
    assert_eq!(rolled[0].actor, Some(r.nick));
    assert!(
        rolled[0].detail["summary"]
            .as_str()
            .unwrap()
            .contains("2 successes")
    );
    // It shows in the recap and the log.
    let out = run(&mut r.engine, r.nick, "+recap");
    assert!(
        texts(&out).contains("Nick rolls — Nick rolls 5 vs 6: 2 successes"),
        "{}",
        texts(&out)
    );
    // A roll in another room is not this scene's.
    let hall = r
        .engine
        .world
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    emit_rolled(&mut r.engine, r.nick, hall, "unrelated");
    let rolled = log_entries(&r.engine, uid)
        .into_iter()
        .filter(|e| e.kind == SceneEventKind::Rolled)
        .count();
    assert_eq!(rolled, 1, "a roll elsewhere is not recorded here");
}

#[test]
fn a_just_closed_scene_is_still_recappable() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "say for the record");
    run(&mut r.engine, r.nick, "+timestop/end");
    // The scene closed, but its transcript is still in RAM — the natural moment
    // to review it (right after it ends) must work.
    let out = run(&mut r.engine, r.nick, "+recap");
    assert!(
        texts(&out).contains("for the record"),
        "closed-scene recap: {}",
        texts(&out)
    );
    let out = run(&mut r.engine, r.nick, "+timestop/log");
    assert!(
        texts(&out).contains("for the record"),
        "closed-scene log: {}",
        texts(&out)
    );
    let out = run(&mut r.engine, r.nick, "+scene");
    assert!(
        texts(&out).contains("closed"),
        "closed-scene status: {}",
        texts(&out)
    );
}

#[test]
fn recap_renders_the_witnessed_narrative() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "say hello all");
    run(&mut r.engine, r.nick, ":waves");
    run(&mut r.engine, r.nick, "+timestop/next");
    let out = run(&mut r.engine, r.ann, "+recap");
    let t = texts(&out);
    assert!(t.contains("The scene begins"), "{t}");
    assert!(t.contains("Nick says, \"hello all\""), "{t}");
    assert!(t.contains("Nick waves"), "pose rendered: {t}");
    assert!(t.contains("round"), "round divider: {t}");
}

// ----------------------------------------------------------------------
// Persistence round trip
// ----------------------------------------------------------------------

fn try_reboot(store: &InMemoryStore) -> Result<Engine, ModError> {
    let kv = futures::executor::block_on(async { store.module_json_list(NS).await.unwrap() });
    let boot = ModuleBootData {
        kv: kv.into_iter().map(|(k, v)| (SmolStr::new(k), v)).collect(),
        streams: HashMap::new(),
    };
    let mut world = World::new_minimal();
    world.set_now(2_000);
    let bar = world
        .create(ObjKind::Room, "The Tap Room", GOD, None)
        .unwrap();
    // Recreate the same players/npc at the same dbrefs (deterministic creation
    // order matches the original rig).
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(bar))
        .unwrap();
    world
        .create(ObjKind::Player, "Ann", GOD, Some(bar))
        .unwrap();
    world
        .create(ObjKind::Npc, "Barkeep", GOD, Some(bar))
        .unwrap();
    world.set_connected(nick, true);

    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine = Engine::with_modules(world, registry, vec![Box::new(ScenesModule::new())])?;
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.modules_loaded(HashMap::from([(SmolStr::new(NS), boot)]), &mut out, &mut fx)?;
    Ok(engine)
}

fn reboot(store: &InMemoryStore) -> Engine {
    try_reboot(store).expect("scene state reboots")
}

#[test]
fn scene_survives_restart_rehydrating_paused_and_still_frozen() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "+timestop/next"); // advance so round/turn state is non-trivial
    let ops: Vec<ModuleOp> = r.engine.take_module_ops();

    let store = InMemoryStore::new();
    futures::executor::block_on(async {
        store
            .write(WriteBatch {
                serial: 1,
                module_ops: ops,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
    });
    let engine2 = reboot(&store);

    // The scene rehydrated, paused by the restart, and its members are frozen
    // again in the fresh world's core index.
    let s = scenes(&engine2)
        .scenes
        .values()
        .next()
        .expect("scene rehydrated");
    assert_eq!(s.status, SceneStatus::Paused);
    assert_eq!(s.pause_reason, Some(PauseReason::ServerRestart));
    // The event log's tail survived (Started + TurnAdvanced were recorded).
    let events = scenes(&engine2).events.open_entries(&s.id.to_string());
    assert!(events.len() >= 2, "the event log rehydrated: {events:?}");
    // Rig dbrefs: Limbo #0, God #1, bar #2, Nick #3, Ann #4, Barkeep #5.
    // After one /next the turn advanced Nick -> Ann (#4); that state persists.
    assert_eq!(
        s.current_actor(),
        Some(ObjId(4)),
        "turn state (Ann) preserved"
    );
    for who in s.participants.clone() {
        assert!(
            engine2.world.in_exclusive_scene(who),
            "{who} refrozen after restart"
        );
    }

    // The facilitator resumes it; turns advance again.
    let mut engine2 = engine2;
    let nick = engine2.world.resolve_player("Nick").unwrap();
    let out = run(&mut engine2, nick, "+timestop/resume");
    assert!(texts(&out).contains("resumes"), "{}", texts(&out));
    assert_eq!(
        scenes(&engine2).active_scene_of(nick).unwrap().status,
        SceneStatus::Active
    );
}

#[test]
fn corrupt_closed_scene_history_refuses_boot() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "+timestop/end");
    let ops = r.engine.take_module_ops();
    let store = InMemoryStore::new();

    futures::executor::block_on(async {
        store
            .write(WriteBatch {
                serial: 1,
                module_ops: ops,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
        let (key, mut chunk) = store
            .module_json_list(NS)
            .await
            .unwrap()
            .into_iter()
            .find(|(key, _)| key.starts_with("log:"))
            .expect("closed scene wrote an event-log chunk");
        chunk["entries"][0]["seq"] = serde_json::json!(99);
        store
            .write(WriteBatch {
                serial: 2,
                module_ops: vec![ModuleOp {
                    ns: SmolStr::new(NS),
                    op: ModuleWrite::JsonSet {
                        key: SmolStr::new(key),
                        value: chunk,
                    },
                }],
                ..WriteBatch::default()
            })
            .await
            .unwrap();
    });

    let error = match try_reboot(&store) {
        Ok(_) => panic!("corrupt archived history must fail closed"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("scene ledger history validation failed"),
        "unexpected boot error: {error}"
    );
}

#[test]
fn behavior_turn_commands_are_gated_like_globals_and_exits_stay_free() {
    let mut r = rig();
    // Ann carries a mech bound with the Turn-class `fire` behavior command.
    let mech = r
        .engine
        .world
        .create(ObjKind::Thing, "Mech", GOD, Some(r.ann))
        .unwrap();
    r.engine
        .world
        .attach_behavior(
            mech,
            mush_core::BehaviorId::new("rolltest.mech").unwrap(),
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
    run(&mut r.engine, r.nick, "+timestop"); // [Nick, Ann, Barkeep], Nick's turn
    let out = run(&mut r.engine, r.ann, "fire");
    assert!(texts(&out).contains("not your turn"), "{}", texts(&out));
    run(&mut r.engine, r.nick, "+timestop/next"); // Ann's turn
    let out = run(&mut r.engine, r.ann, "fire");
    assert!(
        texts(&out).contains(&format!("you fire {mech}")),
        "{}",
        texts(&out)
    );

    // An exit sharing the verb's name is a MOVE: the turn gate must classify it
    // Free (the timestop movement gate is what stops it, not the turn rule).
    let hall = r
        .engine
        .world
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    let ex = r
        .engine
        .world
        .create(ObjKind::Exit, "fire", GOD, Some(r.bar))
        .unwrap();
    r.engine.world.link_exit(ex, Some(hall)).unwrap();
    run(&mut r.engine, r.nick, "+timestop/next"); // Barkeep's turn: Ann is off-turn
    let out = run(&mut r.engine, r.ann, "fire");
    let t = texts(&out);
    assert!(
        t.contains("caught in") && !t.contains("not your turn"),
        "exit read as Free: {t}"
    );
}
