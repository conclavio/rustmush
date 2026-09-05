//! End-to-end mush-script tests through the real [`Engine`].
//!
//! Every scenario drives scripts exactly as the game would: attributes set
//! through `@set` (which recompiles via the additive `AttrSet` signal),
//! commands and speech dispatched through the engine, timers fired through
//! the scheduler. Assertions read the outbox — no internals are poked.

use std::collections::HashMap;

use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::{
    Attr, AttrFlags, AttrValue, Audience, DeliveryClass, Flags, GOD, LIMBO, ObjId, ObjKind, Outbox,
    Registry, World, register_core_commands, register_engine_commands,
};
use mush_proto::OutputEvent;
use mush_script::{ScriptConfig, ScriptModule};

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

struct Fixture {
    engine: Engine,
}

impl Fixture {
    fn new() -> Fixture {
        Fixture::with_config(ScriptConfig::default())
    }

    fn with_config(cfg: ScriptConfig) -> Fixture {
        let mut world = World::new_minimal();
        world.set_now(1_000);
        let mut registry = Registry::new();
        register_core_commands(&mut registry).unwrap();
        register_engine_commands(&mut registry).unwrap();
        let mut engine = Engine::with_modules(
            world,
            registry,
            vec![Box::new(ScriptModule::with_config(cfg))],
        )
        .unwrap();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine
            .modules_loaded(HashMap::new(), &mut out, &mut fx)
            .unwrap();
        Fixture { engine }
    }

    fn player(&mut self, name: &str) -> ObjId {
        self.engine
            .world
            .create(ObjKind::Player, name, GOD, Some(LIMBO))
            .unwrap()
    }

    fn thing(&mut self, name: &str, owner: ObjId, loc: ObjId) -> ObjId {
        self.engine
            .world
            .create(ObjKind::Thing, name, owner, Some(loc))
            .unwrap()
    }

    fn room(&mut self, name: &str) -> ObjId {
        self.engine
            .world
            .create(ObjKind::Room, name, GOD, None)
            .unwrap()
    }

    /// Set a `script:*` attribute directly, then let the module compile it
    /// (as if loaded at boot). Bypasses `@set` for scripts whose text is
    /// awkward to type on a command line.
    fn seed_script(&mut self, obj: ObjId, key: &str, code: &str) {
        let owner = self.engine.world.get(obj).unwrap().owner;
        self.engine
            .world
            .set_attr(obj, key, Attr::text(code, owner))
            .unwrap();
        // Trigger a recompile via the AttrSet observer path.
        self.run(GOD, &format!("@set {obj}/{key}={code}"));
    }

    fn run(&mut self, actor: ObjId, line: &str) -> Vec<(Audience, OutputEvent)> {
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        self.engine.step(
            Inbound::Cmd {
                conn: 1,
                actor,
                line: line.into(),
            },
            &mut out,
            &mut fx,
        );
        out.take()
    }

    fn run_classified(
        &mut self,
        actor: ObjId,
        line: &str,
    ) -> Vec<(DeliveryClass, Audience, OutputEvent)> {
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        self.engine.step(
            Inbound::Cmd {
                conn: 1,
                actor,
                line: line.into(),
            },
            &mut out,
            &mut fx,
        );
        out.take_classified()
    }

    fn run_fx(&mut self, actor: ObjId, line: &str) -> (Vec<(Audience, OutputEvent)>, Effects) {
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        self.engine.step(
            Inbound::Cmd {
                conn: 1,
                actor,
                line: line.into(),
            },
            &mut out,
            &mut fx,
        );
        (out.take(), fx)
    }

    fn set_now(&mut self, secs: i64) {
        self.engine.set_now(secs);
    }

    fn fire_due(&mut self, now_secs: i64) -> Vec<(Audience, OutputEvent)> {
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        self.engine.fire_due(now_secs * 1_000, &mut out, &mut fx);
        out.take()
    }

    fn flag(&self, obj: ObjId, flag: Flags) -> bool {
        self.engine
            .world
            .get(obj)
            .is_some_and(|o| o.flags.contains(flag))
    }

    fn desc(&self, obj: ObjId) -> String {
        self.engine
            .world
            .attr(obj, "desc")
            .and_then(|a| a.as_text().map(str::to_string))
            .unwrap_or_default()
    }
}

#[test]
fn script_commands_and_dollar_fallback_are_declared_legacy_compatibility() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.engine.registry.compatibility_of("use"),
        Some(mush_core::CommandCompatibility::LegacyPostImage)
    );
    assert_eq!(
        fixture
            .engine
            .registry
            .command_fallback_compatibilities()
            .collect::<Vec<_>>(),
        vec![(
            Some("script"),
            mush_core::CommandCompatibility::LegacyPostImage
        )]
    );
}

fn all_text(events: &[(Audience, OutputEvent)]) -> Vec<String> {
    events
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Sys(t) => Some(t.to_plain_text()),
            OutputEvent::Narrative { text, .. } => Some(text.to_plain_text()),
            _ => None,
        })
        .collect()
}

fn saw(events: &[(Audience, OutputEvent)], needle: &str) -> bool {
    all_text(events).iter().any(|t| t.contains(needle))
}

fn actor_saw(events: &[(Audience, OutputEvent)], actor: ObjId, needle: &str) -> bool {
    events.iter().any(|(aud, e)| {
        matches!(aud, Audience::Actor(a) if *a == actor)
            && matches!(e,
                OutputEvent::Narrative { text, .. } | OutputEvent::Sys(text)
                if text.to_plain_text().contains(needle))
    })
}

fn classified_actor_saw(
    events: &[(DeliveryClass, Audience, OutputEvent)],
    class: DeliveryClass,
    actor: ObjId,
    needle: &str,
) -> bool {
    events.iter().any(|(actual_class, aud, event)| {
        *actual_class == class
            && matches!(aud, Audience::Actor(a) if *a == actor)
            && matches!(event,
                OutputEvent::Narrative { text, .. } | OutputEvent::Sys(text)
                if text.to_plain_text().contains(needle))
    })
}

fn count(events: &[(Audience, OutputEvent)], needle: &str) -> usize {
    all_text(events)
        .iter()
        .filter(|t| t.contains(needle))
        .count()
}

// ----------------------------------------------------------------------
// Compile / cache / recompile + a $-command
// ----------------------------------------------------------------------

#[test]
fn command_greet_pemits_caller_and_recompiles_on_change() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let bell = f.thing("bell", nick, LIMBO);
    f.seed_script(
        bell,
        "script:command:greet",
        r#"mush::pemit(mush::caller(), "Hi " + arg)"#,
    );

    let out = f.run(nick, "greet Bob");
    assert!(actor_saw(&out, nick, "Hi Bob"), "{:?}", all_text(&out));

    // Recompile-on-change: edit the script, new behavior takes effect.
    f.run(
        GOD,
        &format!(r#"@set {bell}/script:command:greet=mush::pemit(mush::caller(), "Yo " + arg)"#),
    );
    let out = f.run(nick, "greet Sam");
    assert!(actor_saw(&out, nick, "Yo Sam"));
    assert!(
        !saw(&f.run(nick, "greet Sam"), "Hi "),
        "old AST must be gone"
    );
}

#[test]
fn unknown_verb_without_script_still_huhs() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let out = f.run(nick, "flibbertigibbet");
    assert!(saw(&out, "Huh?"), "no fallback should not swallow Huh");
}

#[test]
fn boot_compiles_existing_scripts() {
    // A script present before the module loads is compiled by on_world_loaded.
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let bell = world
        .create(ObjKind::Thing, "bell", nick, Some(LIMBO))
        .unwrap();
    world
        .set_attr(
            bell,
            "script:command:ping",
            Attr::text(r#"mush::pemit(mush::caller(), "pong")"#, nick),
        )
        .unwrap();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine =
        Engine::with_modules(world, registry, vec![Box::new(ScriptModule::new())]).unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .unwrap();

    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::Cmd {
            conn: 1,
            actor: nick,
            line: "ping".into(),
        },
        &mut out,
        &mut fx,
    );
    assert!(actor_saw(out.events(), nick, "pong"));
}

// ----------------------------------------------------------------------
// Hooks: enter / hear / use / timer
// ----------------------------------------------------------------------

#[test]
fn on_enter_announces_arrivals() {
    let mut f = Fixture::new();
    let hall = f.room("Hall");
    let foyer = f.room("Foyer");
    // Exit Foyer -> Hall.
    f.engine
        .world
        .create(ObjKind::Exit, "north;n", GOD, Some(foyer))
        .unwrap();
    let exit = f.engine.world.exits_of(foyer)[0];
    f.engine.world.link_exit(exit, Some(hall)).unwrap();

    let nick = f.player("Nick");
    f.engine.world.move_to(nick, foyer).unwrap();

    f.seed_script(
        hall,
        "script:enter",
        r#"mush::emit(mush::me(), mush::name(mover) + " strolls in.")"#,
    );

    let out = f.run(nick, "north");
    assert!(saw(&out, "Nick strolls in."), "{:?}", all_text(&out));
}

#[test]
fn on_hear_reacts_to_keyword() {
    let mut f = Fixture::new();
    let doorbell = f.thing("doorbell", GOD, LIMBO);
    let nick = f.player("Nick");
    f.seed_script(
        doorbell,
        "script:hear:ring",
        r#"if text.contains("hello") { mush::emit(mush::here(), "The doorbell chimes!") }"#,
    );

    // A line without the keyword does nothing.
    let out = f.run(nick, "say goodbye");
    assert!(!saw(&out, "chimes"));

    // The keyword triggers the reflex.
    let out = f.run(nick, "say well hello there");
    assert!(saw(&out, "The doorbell chimes!"), "{:?}", all_text(&out));
}

#[test]
fn on_use_runs_use_script_else_refuses() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let lever = f.thing("lever", GOD, LIMBO);
    let rock = f.thing("rock", GOD, LIMBO);
    f.seed_script(
        lever,
        "script:use",
        r#"mush::pemit(user, "The lever clunks.")"#,
    );

    let out = f.run(nick, "use lever");
    assert!(actor_saw(&out, nick, "The lever clunks."));

    let out = f.run(nick, "use rock");
    assert!(actor_saw(&out, nick, "You can't use that."));
    let _ = rock;
}

#[test]
fn on_timer_fires_via_schedule_and_manual_clock() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let clock = f.thing("clock", nick, LIMBO);
    f.seed_script(clock, "script:command:arm", r#"mush::schedule(5, "boom")"#);
    f.seed_script(
        clock,
        "script:timer:boom",
        r#"mush::emit(mush::here(), "BOOM")"#,
    );

    // Arm it: nothing fires yet, a WaitCmd is queued.
    let (out, _fx) = f.run_fx(nick, "arm");
    assert!(!saw(&out, "BOOM"));

    // Before the deadline: still nothing.
    f.set_now(1_003);
    assert!(!saw(&f.fire_due(1_003), "BOOM"));

    // At the deadline the timer fires as the object and emits.
    f.set_now(1_005);
    let out = f.fire_due(1_005);
    assert!(saw(&out, "BOOM"), "{:?}", all_text(&out));
}

// ----------------------------------------------------------------------
// Permission enforcement (as the OWNER, not the trigger)
// ----------------------------------------------------------------------

#[test]
fn mortal_script_cannot_set_attr_on_uncontrolled_object() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let bob = f.player("Bob");
    let vault = f.thing("vault", bob, LIMBO); // owned by Bob
    f.engine
        .world
        .set_attr(vault, "desc", Attr::text("intact", GOD))
        .unwrap();
    let tool = f.thing("tool", nick, LIMBO); // Nick's scripted object
    f.seed_script(
        tool,
        "script:command:hack",
        r#"mush::set_attr(mush::matchit("vault"), "desc", "pwned")"#,
    );

    let out = f.run(nick, "hack");
    assert_eq!(f.desc(vault), "intact", "write must be refused");
    assert!(
        actor_saw(&out, nick, "failed"),
        "owner is told: {:?}",
        all_text(&out)
    );
    let _ = bob;
}

#[test]
fn mortal_script_cannot_move_a_wizards_object() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let idol = f.thing("idol", GOD, LIMBO); // wizard-owned
    let tool = f.thing("tool", nick, LIMBO);
    f.seed_script(
        tool,
        "script:command:grab",
        r#"mush::move(mush::matchit("idol"), mush::me())"#,
    );

    let out = f.run(nick, "grab");
    assert_eq!(
        f.engine.world.get(idol).unwrap().location,
        Some(LIMBO),
        "the idol must not move"
    );
    assert!(actor_saw(&out, nick, "failed"));
}

#[test]
fn owner_script_can_set_attr_on_its_own_object() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let diary = f.thing("diary", nick, LIMBO);
    f.seed_script(
        diary,
        "script:command:write",
        r#"mush::set_attr(mush::me(), "desc", "Dear diary")"#,
    );
    f.run(nick, "write");
    assert_eq!(f.desc(diary), "Dear diary");
}

// ----------------------------------------------------------------------
// Budget enforcement
// ----------------------------------------------------------------------

#[test]
fn infinite_loop_is_aborted_by_operation_cap() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let bomb = f.thing("bomb", nick, LIMBO);
    f.seed_script(
        bomb,
        "script:command:spin",
        "let x = 0; while true { x += 1; }",
    );
    let out = f.run(nick, "spin");
    assert!(actor_saw(&out, nick, "failed"), "{:?}", all_text(&out));
}

#[test]
fn huge_string_is_aborted_by_memory_cap() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let bomb = f.thing("bomb", nick, LIMBO);
    f.seed_script(
        bomb,
        "script:command:grow",
        r#"let s = "x"; while true { s += s; }"#,
    );
    let out = f.run(nick, "grow");
    assert!(actor_saw(&out, nick, "failed"));
}

#[test]
fn deep_recursion_is_aborted_by_call_level_cap() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let bomb = f.thing("bomb", nick, LIMBO);
    f.seed_script(bomb, "script:command:dive", "fn dive() { dive(); } dive();");
    let out = f.run(nick, "dive");
    assert!(actor_saw(&out, nick, "failed"));
}

#[test]
fn wall_clock_backstop_aborts_a_slow_script() {
    let cfg = ScriptConfig {
        wall_clock_ms: 5,
        max_operations: 5_000_000_000,
        ..ScriptConfig::default()
    };
    let mut f = Fixture::with_config(cfg);
    let nick = f.player("Nick");
    let bomb = f.thing("bomb", nick, LIMBO);
    f.seed_script(
        bomb,
        "script:command:stall",
        "let x = 0; while true { x += 1; }",
    );
    let out = f.run(nick, "stall");
    assert!(actor_saw(&out, nick, "failed"), "wall clock must trip");
}

#[test]
fn compile_error_disables_only_that_script_and_notifies_owner() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let obj = f.thing("obj", nick, LIMBO);
    // A good script and a broken one on the same object.
    f.seed_script(
        obj,
        "script:command:good",
        r#"mush::pemit(mush::caller(), "ok")"#,
    );
    let out = f.run_classified(
        GOD,
        &format!("@set {obj}/script:command:bad=this is (not valid rhai"),
    );
    assert!(
        classified_actor_saw(&out, DeliveryClass::OnCommit, nick, "failed to compile"),
        "a compile diagnostic accompanies the committed attribute change"
    );
    // The good one still runs; the bad one just Huhs (never compiled).
    assert!(actor_saw(&f.run(nick, "good"), nick, "ok"));
    assert!(saw(&f.run(nick, "bad blah"), "Huh?"));
}

// ----------------------------------------------------------------------
// 3-strike HALTED auto-disable
// ----------------------------------------------------------------------

#[test]
fn three_budget_overruns_halt_the_object() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let bomb = f.thing("bomb", nick, LIMBO);
    f.seed_script(bomb, "script:command:spin", "while true {}");

    for _ in 0..2 {
        let out = f.run(nick, "spin");
        assert!(actor_saw(&out, nick, "failed"));
        assert!(!f.flag(bomb, Flags::HALTED));
    }
    // Third overrun trips the auto-disable.
    let out = f.run_classified(nick, "spin");
    assert!(
        classified_actor_saw(&out, DeliveryClass::OnCommit, nick, "failed"),
        "the third failure report must wait for the HALTED write"
    );
    assert!(
        classified_actor_saw(&out, DeliveryClass::OnCommit, nick, "HALTED"),
        "the HALTED claim must be released only after its write commits"
    );
    assert!(f.flag(bomb, Flags::HALTED));

    // A HALTED object's scripts no longer run at all.
    let out = f.run(nick, "spin");
    assert!(
        saw(&out, "Huh?"),
        "HALTED object is inert: {:?}",
        all_text(&out)
    );
}

// ----------------------------------------------------------------------
// Re-entrancy cap
// ----------------------------------------------------------------------

#[test]
fn cascade_cap_stops_an_emit_hear_ping_pong() {
    let cfg = ScriptConfig {
        max_total_invocations: 8,
        ..ScriptConfig::default()
    };
    let mut f = Fixture::with_config(cfg);
    let nick = f.player("Nick");
    let a = f.thing("alpha", GOD, LIMBO);
    let b = f.thing("beta", GOD, LIMBO);
    f.seed_script(a, "script:hear:x", r#"mush::emit(mush::here(), "ping")"#);
    f.seed_script(b, "script:hear:x", r#"mush::emit(mush::here(), "pong")"#);

    // Without a cap this never terminates; with one it must, and be bounded.
    let out = f.run(nick, "say start");
    let pings = count(&out, "ping");
    let pongs = count(&out, "pong");
    let total = pings + pongs;
    assert!(total >= 3, "the cascade actually cascaded: {total}");
    assert!(total <= 8, "the cascade is bounded by the cap: {total}");
}

// ----------------------------------------------------------------------
// API surface + ObjId round-trip
// ----------------------------------------------------------------------

#[test]
fn there_is_no_mush_cmd() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let obj = f.thing("obj", nick, LIMBO);
    f.seed_script(obj, "script:command:sneaky", r#"mush::cmd("look")"#);
    let out = f.run(nick, "sneaky");
    // The call errors (no such function) and no room render happens.
    assert!(actor_saw(&out, nick, "failed"));
    assert!(
        !out.iter().any(|(_, e)| matches!(e, OutputEvent::Room(_))),
        "mush::cmd must not have run a command"
    );
}

#[test]
fn objid_round_trips_through_rhai() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let mirror = f.thing("mirror", nick, LIMBO);
    f.seed_script(
        mirror,
        "script:command:whoami",
        r#"mush::pemit(mush::caller(), "me=" + to_string(mush::me()) + " caller=" + to_string(mush::caller()))"#,
    );
    let out = f.run(nick, "whoami");
    let expected = format!("me={mirror} caller={nick}");
    assert!(actor_saw(&out, nick, &expected), "{:?}", all_text(&out));

    // Equality of ObjId values works too.
    f.seed_script(
        mirror,
        "script:command:same",
        r#"if mush::here() == mush::loc(mush::me()) { mush::pemit(mush::caller(), "yes") } else { mush::pemit(mush::caller(), "no") }"#,
    );
    assert!(actor_saw(&f.run(nick, "same"), nick, "yes"));
}

#[test]
fn dice_rolls_within_bounds() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let die = f.thing("die", nick, LIMBO);
    f.seed_script(
        die,
        "script:command:roll",
        r#"mush::pemit(mush::caller(), "r=" + (mush::dice("2d6+1")).to_string())"#,
    );
    // Several rolls, all within [3, 13].
    for _ in 0..12 {
        let out = f.run(nick, "roll");
        let line = all_text(&out)
            .into_iter()
            .find(|t| t.starts_with("r="))
            .unwrap();
        let n: i64 = line.trim_start_matches("r=").parse().unwrap();
        assert!((3..=13).contains(&n), "2d6+1 out of range: {n}");
    }
}

#[test]
fn command_scripts_are_a_fallback_not_a_shadow() {
    // A script:command:say must NOT capture the built-in `say`.
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let obj = f.thing("obj", nick, LIMBO);
    f.seed_script(
        obj,
        "script:command:say",
        r#"mush::pemit(mush::caller(), "HIJACKED")"#,
    );
    let out = f.run(nick, "say hello");
    assert!(saw(&out, "hello"), "built-in say still runs");
    assert!(!saw(&out, "HIJACKED"), "script must not shadow a built-in");
}

#[test]
fn command_script_never_shadows_an_exit() {
    // Dispatch matches exits (step 4) before the script fallback (step 5): a
    // `script:command:north` must never fire while a `north` exit exists.
    let mut f = Fixture::new();
    let hall = f.room("Hall");
    let foyer = f.room("Foyer");
    f.engine
        .world
        .create(ObjKind::Exit, "north;n", GOD, Some(foyer))
        .unwrap();
    let exit = f.engine.world.exits_of(foyer)[0];
    f.engine.world.link_exit(exit, Some(hall)).unwrap();
    let nick = f.player("Nick");
    f.engine.world.move_to(nick, foyer).unwrap();
    // A scripted object in the room offers a rival `north` $-command.
    let gadget = f.thing("gadget", nick, foyer);
    f.seed_script(
        gadget,
        "script:command:north",
        r#"mush::pemit(mush::caller(), "SHADOW")"#,
    );

    let out = f.run(nick, "north");
    assert_eq!(
        f.engine.world.get(nick).unwrap().location,
        Some(hall),
        "the exit wins over the script fallback"
    );
    assert!(
        !saw(&out, "SHADOW"),
        "script:command must not shadow an exit: {:?}",
        all_text(&out)
    );
}

// ----------------------------------------------------------------------
// Sandbox-safety regressions (M5 review)
// ----------------------------------------------------------------------

#[test]
fn dice_non_ascii_or_overflow_fails_without_panicking() {
    // Regression (SEV-1): a non-ASCII first char used to slice mid-codepoint
    // and panic the whole world task; an extreme modifier used to
    // overflow-panic in a debug build. Both must now be plain, catchable
    // script errors — and reaching the assertions at all proves no panic.
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let die = f.thing("die", nick, LIMBO);
    f.seed_script(
        die,
        "script:command:roll",
        r#"mush::pemit(mush::caller(), "r=" + (mush::dice(arg)).to_string())"#,
    );
    for bad in [
        "\u{a3}5",
        "\u{d7}2",
        "\u{b2}",
        "\u{df}d6",
        "1d1+9223372036854775807",
    ] {
        let out = f.run(nick, &format!("roll {bad}"));
        assert!(
            actor_saw(&out, nick, "failed"),
            "bad dice {bad:?} should fail gracefully: {:?}",
            all_text(&out)
        );
    }
    // A valid roll still works afterward (the object survived).
    let out = f.run(nick, "roll 2d6+1");
    assert!(actor_saw(&out, nick, "r="), "{:?}", all_text(&out));
}

#[test]
fn on_enter_reflex_fires_exactly_once_per_move() {
    // A double-fire (Moved + a manual collect, or a doubly-registered
    // observer) would still leave the token present; assert the *count*.
    let mut f = Fixture::new();
    let hall = f.room("Hall");
    let foyer = f.room("Foyer");
    f.engine
        .world
        .create(ObjKind::Exit, "north;n", GOD, Some(foyer))
        .unwrap();
    let exit = f.engine.world.exits_of(foyer)[0];
    f.engine.world.link_exit(exit, Some(hall)).unwrap();
    let nick = f.player("Nick");
    f.engine.world.move_to(nick, foyer).unwrap();
    f.seed_script(
        hall,
        "script:enter",
        r#"mush::emit(mush::me(), "ARRIVED-ONCE-TOKEN")"#,
    );

    let out = f.run(nick, "north");
    assert_eq!(
        count(&out, "ARRIVED-ONCE-TOKEN"),
        1,
        "the enter reflex fires exactly once: {:?}",
        all_text(&out)
    );
}

#[test]
fn script_authored_script_compiles_inline() {
    // A script writing a `script:*` attr recompiles inline, so a script
    // authored by a script is live immediately (not only after a reboot).
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let obj = f.thing("obj", nick, LIMBO);
    f.seed_script(
        obj,
        "script:command:author",
        r#"mush::set_attr(mush::me(), "script:command:foo", "mush::pemit(mush::caller(), arg)")"#,
    );
    // Before authoring, `foo` is unknown.
    assert!(saw(&f.run(nick, "foo hi"), "Huh?"));
    // Author it, then it runs immediately.
    f.run(nick, "author");
    let out = f.run(nick, "foo LIVE");
    assert!(
        actor_saw(&out, nick, "LIVE"),
        "inline recompile makes the new script live: {:?}",
        all_text(&out)
    );
}

#[test]
fn get_attr_withholds_hidden_attrs_from_uncontrolled_reader() {
    // SEV-2: HIDDEN (and other restricted) attrs must not be readable by a
    // script whose owner cannot examine the object; plain attrs stay readable.
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let bob = f.player("Bob");
    let vault = f.thing("vault", bob, LIMBO); // Bob's — Nick can't examine
    f.engine
        .world
        .set_attr(
            vault,
            "secret",
            Attr {
                value: AttrValue::Text("password123".into()),
                owner: bob,
                flags: AttrFlags::HIDDEN,
            },
        )
        .unwrap();
    f.engine
        .world
        .set_attr(vault, "public", Attr::text("hello", bob))
        .unwrap();
    let tool = f.thing("tool", nick, LIMBO);
    f.seed_script(
        tool,
        "script:command:peek",
        r#"mush::pemit(mush::caller(), "s=[" + mush::get_attr(mush::matchit("vault"), "secret") + "] p=[" + mush::get_attr(mush::matchit("vault"), "public") + "]")"#,
    );

    let out = f.run(nick, "peek");
    assert!(
        actor_saw(&out, nick, "s=[]"),
        "HIDDEN attr must be withheld: {:?}",
        all_text(&out)
    );
    assert!(
        actor_saw(&out, nick, "p=[hello]"),
        "a plain attr stays readable: {:?}",
        all_text(&out)
    );
    let _ = bob;
}

#[test]
fn contents_omits_dark_members_a_script_cannot_see() {
    // SEV-3: contents honors DARK visibility — a DARK member the owner does
    // not control is omitted, exactly as the look layer would hide it.
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let bin = f.thing("bin", nick, LIMBO); // Nick controls the container
    let plain = f.thing("plain", GOD, LIMBO);
    let dark = f.thing("dark", GOD, LIMBO);
    f.engine.world.move_to(plain, bin).unwrap();
    f.engine.world.move_to(dark, bin).unwrap();
    f.engine.world.set_flag(dark, Flags::DARK, true).unwrap();
    let tool = f.thing("tool", nick, LIMBO);
    f.seed_script(
        tool,
        "script:command:peek",
        r#"mush::pemit(mush::caller(), "n=" + mush::contents(mush::matchit("bin")).len().to_string())"#,
    );

    let out = f.run(nick, "peek");
    assert!(
        actor_saw(&out, nick, "n=1"),
        "the DARK member is omitted (only 'plain' is visible): {:?}",
        all_text(&out)
    );
}

#[test]
fn script_set_attr_respects_wizard_only_and_preserves_flags() {
    // SEV-4: mush::set_attr must not overwrite a WIZARD_ONLY attr as a
    // non-Wizard, and must carry existing attr flags forward on rewrite.
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let obj = f.thing("obj", nick, LIMBO); // Nick owns it
    // Staff stored a WIZARD_ONLY attr; Nick set a HIDDEN note of his own.
    f.engine
        .world
        .set_attr(
            obj,
            "staff",
            Attr {
                value: AttrValue::Text("secret".into()),
                owner: GOD,
                flags: AttrFlags::WIZARD_ONLY,
            },
        )
        .unwrap();
    f.engine
        .world
        .set_attr(
            obj,
            "note",
            Attr {
                value: AttrValue::Text("v1".into()),
                owner: nick,
                flags: AttrFlags::HIDDEN,
            },
        )
        .unwrap();
    f.seed_script(
        obj,
        "script:command:clobber",
        r#"mush::set_attr(mush::me(), "staff", "pwned")"#,
    );
    f.seed_script(
        obj,
        "script:command:edit",
        r#"mush::set_attr(mush::me(), "note", "v2")"#,
    );

    // The WIZARD_ONLY overwrite is refused; the value is unchanged.
    f.run(nick, "clobber");
    assert_eq!(
        f.engine
            .world
            .attr(obj, "staff")
            .and_then(|a| a.as_text().map(str::to_string)),
        Some("secret".to_string()),
        "a non-Wizard script must not overwrite a WIZARD_ONLY attr"
    );

    // Rewriting Nick's own HIDDEN note is allowed and keeps the HIDDEN flag.
    f.run(nick, "edit");
    let note = f.engine.world.get(obj).unwrap().attrs.get("note").cloned();
    let note = note.expect("note attr exists");
    assert_eq!(note.as_text(), Some("v2"));
    assert!(
        note.flags.contains(AttrFlags::HIDDEN),
        "existing attr flags must be carried forward, not stripped"
    );
}
